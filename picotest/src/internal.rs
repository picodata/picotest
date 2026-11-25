//! Picotest Internal API
//!
//! Contains helper routines called by proc macro unfolding.
//! This module isn't supposed to be used manually.

use anyhow::bail;
use picotest_helpers::migration::{
    find_migrations_directories, make_ddl_tier_overrides, parse_migrations,
};
use picotest_helpers::topology::{
    parse_topology, PluginTopology, SingleNodeTopologyTransformer, Tier, TopologyTransformer,
    DEFAULT_TIER,
};
use picotest_helpers::{cargo_target_dir, use_release_build, Cluster, DEFAULT_WAIT_VSHARD_ENABLED};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::env::{var, VarError};
use std::process::{Command, Stdio};
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
const ENV_PLUGIN_PATH: &str = "PLUGIN_PATH";
const ENV_TOPOLOGY_PATH: &str = "TOPOLOGY_PATH";

const STANDALONE_CLUSTER_KEY: &str = "standalone";

/// Returns the directory where the plugin is built in the selected profile.
pub fn plugin_profile_build_path(plugin_path: &Path) -> PathBuf {
    let profile = if use_release_build() {
        "release"
    } else {
        "debug"
    };
    cargo_target_dir()
        .unwrap_or_else(|| plugin_path.join("target"))
        .join(profile)
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
/// Resolution order:
/// 1. `PLUGIN_PATH` environment variable, if set and the path exists.
/// 2. Walk ancestors of `CARGO_MANIFEST_DIR` looking for `topology.toml`,
///    then take its parent directory.
///
/// Panics if it was not found.
///
/// Basically, it looks for topology.toml file and then
/// returns its parent directory.
pub fn plugin_root_dir() -> PathBuf {
    let plugin_root_dir = var(ENV_PLUGIN_PATH)
        .map(PathBuf::from)
        .ok()
        .filter(|path| path.exists())
        .unwrap_or_else(|| {
            find_plugin_topology_path()
                .expect("Error occurred while searching for plugin topology configuration")
                .expect("Plugin topology configuration is not found")
                .parent()
                .expect("Failed to obtain parent directory of plugin topology file")
                .to_path_buf()
        });

    assert!(
        plugin_root_dir.join("Cargo.toml").exists(),
        "Plugin root directory does not contain Cargo.toml: {}",
        plugin_root_dir.display()
    );

    plugin_root_dir
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
/// - `plugin_topology` - instance of `PluginTopology`.
///   If `None`, topology is parsed from default path.
///
pub fn create_cluster(plugin_topology: Option<PluginTopology>) -> Cluster {
    // Look up plugin root directory (see `plugin_root_dir`).
    let plugin_path = plugin_root_dir();
    // Use passed topology or go and parse original topology
    // located in plugin root directory.
    let plugin_topology = plugin_topology.map_or_else(
        || parse_topology(&plugin_topology_path(&plugin_path)),
        Result::Ok,
    );

    start_cluster(plugin_path, plugin_topology.unwrap())
}

/// Starts a cluster of `plugin_topology` with `plugin_path` as the plugin directory for pike.
fn start_cluster(plugin_path: PathBuf, plugin_topology: PluginTopology) -> Cluster {
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

    Cluster::new(plugin_path, plugin_topology, picodata_path)
        .expect("Failed to create the cluster")
        .wait_vshard_discovery(wait_vshard_discovery)
        .run()
        .expect("Failed to start the cluster")
}

/// Runs a test generated by `#[picotest_unit]`.
///
/// The test runs in plugin mode if the crate is a part of a plugin: `PLUGIN_PATH`
/// points to an existing directory, or `topology.toml` is found in `manifest_dir`
/// or one of its ancestors. Otherwise it runs in standalone mode, see
/// [`run_standalone_unit_test`].
///
/// ### Arguments
/// - `ffi_test_callable` - name of the exported symbol of the test routine.
/// - `test_fn_name` - name of the test as written by the user.
/// - `package_name` - `CARGO_PKG_NAME` of the crate under test.
/// - `manifest_dir` - `CARGO_MANIFEST_DIR` of the crate under test.
pub fn run_unit_test(
    ffi_test_callable: &str,
    test_fn_name: &str,
    package_name: &str,
    manifest_dir: &str,
) {
    if is_plugin_crate(Path::new(manifest_dir)) {
        run_plugin_unit_test(ffi_test_callable, test_fn_name, package_name);
    } else {
        run_standalone_unit_test(ffi_test_callable, test_fn_name, Path::new(manifest_dir));
    }
}

fn is_plugin_crate(manifest_dir: &Path) -> bool {
    let plugin_path_is_set = var(ENV_PLUGIN_PATH).is_ok_and(|path| Path::new(&path).exists());

    plugin_path_is_set
        || manifest_dir
            .ancestors()
            .any(|path| path.join(PLUGIN_TOPOLOGY_FILENAME).exists())
}

fn run_plugin_unit_test(ffi_test_callable: &str, test_fn_name: &str, package_name: &str) {
    let plugin_path = plugin_root_dir();
    let plugin_dylib_path = plugin_dylib_path(&plugin_path, package_name);
    let plugin_topology = get_or_create_unit_test_topology();

    let cluster = crate::get_or_create_session_cluster(plugin_topology.into());

    run_unit_test_on(cluster, ffi_test_callable, test_fn_name, &plugin_dylib_path);
}

/// Runs a unit-test of a crate that is not a plugin.
///
/// The cluster is a single picodata instance without plugins, started by pike
/// with `manifest_dir` as the plugin directory. Since it's not a plugin directory,
/// pike neither builds nor loads anything, so the crate is built here.
///
/// Like in plugin mode, tests of one binary run in parallel and share the instance.
fn run_standalone_unit_test(ffi_test_callable: &str, test_fn_name: &str, manifest_dir: &Path) {
    // A test binary contains tests of one crate, so one library is enough.
    static DYLIB_PATH: OnceLock<PathBuf> = OnceLock::new();

    let dylib_path = DYLIB_PATH.get_or_init(|| build_crate_dylib(manifest_dir));

    let key = (
        manifest_dir.to_path_buf(),
        STANDALONE_CLUSTER_KEY.to_string(),
    );
    let cluster = crate::get_or_create_cluster(key, || {
        start_cluster(manifest_dir.to_path_buf(), standalone_topology())
    });

    run_unit_test_on(cluster, ffi_test_callable, test_fn_name, dylib_path);
}

/// Topology of a single instance in the default tier and no plugins.
fn standalone_topology() -> PluginTopology {
    PluginTopology {
        tiers: BTreeMap::from([(
            DEFAULT_TIER.to_string(),
            Tier {
                replicasets: 1,
                replication_factor: 1,
            },
        )]),
        ..Default::default()
    }
}

/// Builds the library of the crate in `manifest_dir` in the profile and target directory
/// of the tests, and returns the path to its dynamic library.
///
/// The path is taken from the build messages of cargo: the target directory of the tests
/// can't always be derived from the path of the test binary.
///
/// Like pike does for plugins, features of the tests aren't passed to this build.
fn build_crate_dylib(manifest_dir: &Path) -> PathBuf {
    let manifest_path = manifest_dir.join("Cargo.toml");
    // Cargo reports canonical paths.
    let manifest_path = std::fs::canonicalize(&manifest_path).unwrap_or(manifest_path);
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("build")
        .arg("--lib")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .stdout(Stdio::piped());
    if use_release_build() {
        cmd.arg("--release");
    }
    if let Some(target_dir) = cargo_target_dir() {
        cmd.arg("--target-dir").arg(target_dir);
    }

    let output = cmd
        .output()
        .unwrap_or_else(|err| panic!("Failed to run {cmd:?}: {err}"));
    assert!(
        output.status.success(),
        "Failed to build the crate: {cmd:?} exited with {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    find_dylib_artifact(&stdout, &manifest_path).unwrap_or_else(|| {
        panic!(
            "Dynamic library of '{}' is not found after the build. \
             Add `crate-type = [\"cdylib\", \"lib\"]` to the [lib] section of Cargo.toml",
            manifest_path.display()
        )
    })
}

/// Finds the dynamic library of the package `manifest_path` in JSON messages of `cargo build`.
fn find_dylib_artifact(messages: &str, manifest_path: &Path) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["manifest_path"].as_str().map(Path::new) == Some(manifest_path))
        .filter_map(|message| message["filenames"].as_array().cloned())
        .flatten()
        .filter_map(|filename| filename.as_str().map(PathBuf::from))
        .find(|filename| filename.extension().is_some_and(|ext| ext == LIB_EXT))
}

fn run_unit_test_on(
    cluster: &Cluster,
    ffi_test_callable: &str,
    test_fn_name: &str,
    dylib_path: &Path,
) {
    let call_test_fn_query =
        lua_ffi_call_unit_test(ffi_test_callable, dylib_path.to_str().unwrap());

    let output = cluster
        .run_lua(call_test_fn_query)
        .expect("Failed to execute query");

    if let Err(err) = verify_unit_test_output(&output) {
        for l in output.split("----") {
            println!("[Lua] {l}")
        }
        panic!("Test '{test_fn_name}' exited with failure: {err}");
    }
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
    use super::{find_dylib_artifact, verify_unit_test_output, LIB_EXT};
    use std::path::{Path, PathBuf};

    #[test]
    fn finds_dylib_of_the_package_in_cargo_messages() {
        let messages = format!(
            r#"{{"reason":"compiler-artifact","manifest_path":"/deps/dep/Cargo.toml","filenames":["/target/debug/libdep.{LIB_EXT}"]}}
not a json line
{{"reason":"compiler-artifact","manifest_path":"/ws/my-crate/Cargo.toml","filenames":["/target/debug/libmy_lib.rlib","/target/debug/libmy_lib.{LIB_EXT}"]}}
{{"reason":"build-finished","success":true}}"#
        );

        assert_eq!(
            find_dylib_artifact(&messages, Path::new("/ws/my-crate/Cargo.toml")),
            Some(PathBuf::from(format!("/target/debug/libmy_lib.{LIB_EXT}")))
        );
        assert_eq!(
            find_dylib_artifact(&messages, Path::new("/ws/other/Cargo.toml")),
            None
        );
    }

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
