//! Picotest Internal API
//!
//! Contains helper routines called by proc macro unfolding.
//! This module isn't supposed to be used manually.

use anyhow::bail;
use picotest_helpers::migration::{
    find_migrations_directories, make_ddl_tier_overrides, parse_migrations,
};
use picotest_helpers::topology::{
    parse_topology, PluginTopology, SingleNodeTopologyTransformer, TopologyTransformer,
    DEFAULT_TIER,
};
use picotest_helpers::{Cluster, DEFAULT_WAIT_VSHARD_ENABLED};
use serde::Deserialize;
use std::collections::HashMap;
use std::env::{var, VarError};
use std::{
    env,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

const PLUGIN_TOPOLOGY_FILENAME: &str = "topology.toml";

const ENV_WAIT_VSHARD_DISCOVERY: &str = "WAIT_VSHARD_DISCOVERY";
const ENV_PICODATA_PATH: &str = "PICODATA_PATH";
const ENV_TOPOLOGY_PATH: &str = "TOPOLOGY_PATH";

pub fn plugin_profile_build_path(plugin_path: &Path) -> PathBuf {
    plugin_path.join("target").join("debug")
}

/// Constructs a path to the shared library of the plugin
/// located by passed `plugin_path`.
pub fn plugin_dylib_path(plugin_path: &Path, package_name: &str) -> PathBuf {
    let plugin_dylib_filename = format!("lib{}.{LIB_EXT}", package_name.replace('-', "_"));
    plugin_profile_build_path(plugin_path).join(plugin_dylib_filename)
}

/// Constructs a path to the topology file of the plugin.
pub fn plugin_topology_path(plugin_path: &Path) -> PathBuf {
    if let Ok(path) = var(ENV_TOPOLOGY_PATH) {
        let topology_path = PathBuf::from(&path);
        if topology_path.exists() {
            return topology_path;
        }
        println!(
            "ENV_TOPOLOGY_PATH environment variable is not set, \
                using default PATH"
        );
    }

    plugin_path.join(PLUGIN_TOPOLOGY_FILENAME)
}

/// Returns root directory of the plugin.
///
/// Panics if it was not found.
///
/// Basically, it looks for topology.toml file and then
/// returns its parent directory.
pub fn plugin_root_dir() -> PathBuf {
    let plugin_topology_path = find_plugin_topology_path()
        .expect("Error occurred while searching for plugin topology configuration")
        .expect("Plugin topology configuration is not found");

    let plugin_root_dir = plugin_topology_path
        .parent()
        .expect("Failed to obtain parent directory of plugin topology file");

    assert!(
        plugin_root_dir.join("Cargo.toml").exists(),
        "broken plugin directory?"
    );

    plugin_root_dir.to_path_buf()
}

/// Finds path to the plugin topology file.
///
/// ### Returns
/// * On success, `Some(path)`, where path is pointing to topology configuration,
///   or `None` if topology configuration was not found.
///
/// * On failure, instance of [`anyhow::Error`] describing occurred failure.
pub fn find_plugin_topology_path() -> anyhow::Result<Option<PathBuf>> {
    let manifest_dir: PathBuf = env::var("CARGO_MANIFEST_DIR")?.into();

    for path in manifest_dir.ancestors() {
        let topology_path = path.join(PLUGIN_TOPOLOGY_FILENAME);

        if topology_path.exists() {
            return Ok(Some(topology_path));
        }
    }

    Ok(None)
}

/// Creates Lua script that does FFI call of provided target function taken
/// from dynamic library.
///
/// This script is supposed to be executed from Picodata environment. E.g.,
/// through admin tty.
///
/// ### Arguments
/// - `test_fn_name` - name of the test function to call dynamically.
/// - `plugin_dylib_path` - path to the plugin shared library, which should
///   contain test function symbol.
///
pub fn lua_ffi_call_unit_test(test_fn_name: &str, plugin_dylib_path: &str) -> String {
    format!(
        r#"
"[*] Running unit-test '{test_fn_name}'"

ffi = require("ffi")
ffi.cdef[[void {test_fn_name}();]]
dylib = "{plugin_dylib_path}"
ffi.load(dylib).{test_fn_name}()

"[*] Test '{test_fn_name}' has been finished"
true"#
    )
}

/// Recursively looks for a Tarantool console `error:` record inside a
/// parsed YAML document, returning its message if found.
///
/// The admin console reports a failed command as a one-element sequence
/// containing a mapping with an `error` key, e.g. `- error: '...'`. This
/// walks sequences to find such a mapping, regardless of the underlying
/// OS-specific wording of the error message (glibc's dlopen/dlsym errors
/// look nothing like dyld's on macOS).
fn find_console_error(document: &serde_norway::Value) -> Option<String> {
    match document {
        serde_norway::Value::Mapping(mapping) => mapping.get("error").map(|error| match error {
            serde_norway::Value::String(message) => message.clone(),
            other => format!("{other:?}"),
        }),
        serde_norway::Value::Sequence(items) => items.iter().find_map(find_console_error),
        _ => None,
    }
}

/// Verifies output of a unit-test run through [`lua_ffi_call_unit_test`].
///
/// The admin console executes the generated Lua script command-by-command,
/// so a failure in one command (e.g. `ffi.load(...)` unable to resolve a
/// symbol) does not stop the remaining commands from running and printing
/// their own output. This means a naive `output.contains("true")` check
/// can't be trusted on its own: the trailing `true` literal in the script
/// always gets printed, even when the actual test body never ran.
///
/// Instead of matching OS-specific error text (glibc's dlopen/dlsym
/// messages differ from macOS's dyld ones), this parses the console
/// output as a stream of YAML documents - one per executed command - and
/// checks whether any of them is a Tarantool console `error:` record.
pub fn verify_unit_test_output(output: &str) -> anyhow::Result<()> {
    for document in serde_norway::Deserializer::from_str(output) {
        let Ok(value) = serde_norway::Value::deserialize(document) else {
            continue;
        };

        if let Some(error_message) = find_console_error(&value) {
            bail!("unit-test routine failed: {error_message}");
        }
    }

    if !output.contains("true") {
        bail!("test has finished unexpectedly")
    }

    Ok(())
}

/// Creates new instance of Picodata [`Cluster`].
///
/// ### Arguments
/// - `plugin_path` - path to the plugin root directory.
///   If `None`, directory is identified automatically.
/// - `plugin_topology` - instance of `PluginTopology`.
///   If `None`, topology is parsed from default path.
///
pub fn create_cluster(
    plugin_path: Option<PathBuf>,
    plugin_topology: Option<PluginTopology>,
) -> Cluster {
    // Look up plugin root directory automatically
    // unless explicitly specified.
    let plugin_path = plugin_path.unwrap_or_else(plugin_root_dir);
    // Use passed topology or go and parse original topology
    // located in plugin root directory.
    let plugin_topology = plugin_topology.map_or_else(
        || parse_topology(&plugin_topology_path(&plugin_path)),
        Result::Ok,
    );

    let picodata_path = var(ENV_PICODATA_PATH)
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            println!(
                "PICODATA_PATH environment variable is not set, \
                using default picodata binary from PATH"
            );
            PathBuf::from("picodata")
        });

    let wait_vshard_discovery = var(ENV_WAIT_VSHARD_DISCOVERY)
        .map(|v| v.parse::<bool>().expect("invalid boolean"))
        .unwrap_or_else(|e| match e {
            VarError::NotPresent => DEFAULT_WAIT_VSHARD_ENABLED,
            _ => panic!("failed to read {ENV_WAIT_VSHARD_DISCOVERY}: {e}"),
        });

    Cluster::new(plugin_path, plugin_topology.unwrap(), picodata_path)
        .expect("Failed to create the cluster")
        .wait_vshard_discovery(wait_vshard_discovery)
        .run()
        .expect("Failed to start the cluster")
}

/// Provides topology specifically for running unit-tests.
///
/// Basically, it takes source plugin topology and transforms it to a
/// single node cluster with only one default tier.
///
/// Note: on first call topology is created. Any consequent calls will
/// only take already initialized value from sync. cell.
///
pub fn get_or_create_unit_test_topology() -> &'static PluginTopology {
    static TOPOLOGY: OnceLock<PluginTopology> = OnceLock::new();

    TOPOLOGY.get_or_init(|| {
        let plugin_root = plugin_root_dir();
        let plugin_topology_path = plugin_topology_path(&plugin_root);
        let plugin_topology = parse_topology(&plugin_topology_path).unwrap();

        let profile_path = plugin_profile_build_path(&plugin_root);
        let migrations_paths = find_migrations_directories(profile_path).unwrap();
        let mut context_vars_map = HashMap::new();
        for (plugin_name, migrations_path) in migrations_paths {
            let plugin_migrations = parse_migrations(&migrations_path).unwrap();
            let ctx_vars = make_ddl_tier_overrides(&plugin_migrations, DEFAULT_TIER);
            context_vars_map.insert(plugin_name, ctx_vars);
        }

        let mut transformer = SingleNodeTopologyTransformer::default();
        transformer.set_migration_context_provider(context_vars_map);
        transformer.transform(&plugin_topology)
    })
}

#[cfg(test)]
mod tests {
    use super::verify_unit_test_output;

    const SUCCESS_OUTPUT: &str = r#"
---
- '[*] Running unit-test ''should_success'''
...
---
- '[*] Test ''should_success'' has been finished'
...
---
- true
...
"#;

    #[test]
    fn accepts_successful_run() {
        assert!(verify_unit_test_output(SUCCESS_OUTPUT).is_ok());
    }

    #[test]
    fn rejects_missing_symbol_error_on_linux() {
        // glibc's dlsym() wording, as observed on Linux CI.
        let output = r#"
---
- '[*] Running unit-test ''missing_symbol'''
...
---
- error: 'builtin/ffi.lua:162: ./libtest.so: undefined symbol: missing_symbol'
...
---
- '[*] Test ''missing_symbol'' has been finished'
...
---
- true
...
"#;

        let err = verify_unit_test_output(output).expect_err(
            "must be detected as failure even though trailing banner and `true` still ran",
        );
        assert!(err.to_string().contains("undefined symbol"));
    }

    #[test]
    fn rejects_missing_symbol_error_on_macos() {
        // dyld's dlsym() wording, as observed on macOS - this is the
        // regression this function used to silently pass as `Ok(())`.
        let output = r#"
---
- '[*] Running unit-test ''missing_symbol'''
...
---
- error: 'builtin/ffi.lua:162: dlsym(RTLD_DEFAULT, missing_symbol): symbol not found'
...
---
- '[*] Test ''missing_symbol'' has been finished'
...
---
- true
...
"#;

        let err = verify_unit_test_output(output).expect_err(
            "must be detected as failure even though trailing banner and `true` still ran",
        );
        assert!(err.to_string().contains("symbol not found"));
    }

    #[test]
    fn rejects_missing_shared_library_on_linux() {
        let output = r#"
---
- '[*] Running unit-test ''missing_lib'''
...
---
- error: 'libtest.so: cannot open shared object file: No such file or directory'
...
---
- '[*] Test ''missing_lib'' has been finished'
...
---
- true
...
"#;

        assert!(verify_unit_test_output(output).is_err());
    }

    #[test]
    fn rejects_missing_shared_library_on_macos() {
        // dyld's wording for a missing shared library differs entirely
        // from glibc's "cannot open shared object file".
        let output = r#"
---
- '[*] Running unit-test ''missing_lib'''
...
---
- error: 'dlopen(./libtest.dylib, 0x0005): tried: ''./libtest.dylib'' (no such file)'
...
---
- '[*] Test ''missing_lib'' has been finished'
...
---
- true
...
"#;

        assert!(verify_unit_test_output(output).is_err());
    }

    #[test]
    fn rejects_output_without_trailing_true() {
        let output = r#"
---
- '[*] Running unit-test ''crashed'''
...
"#;

        assert!(verify_unit_test_output(output).is_err());
    }
}
