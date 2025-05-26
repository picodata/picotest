//! Picotest Internal API
//!
//! Contains helper routines called by proc macro unfolding.
//! This module isn't supposed to be used manually.

use anyhow::bail;
use picotest_helpers::{
    topology::{
        parse_topology, PluginTopology, SingleNodeTopologyTransformer, TopologyTransformer,
    },
    Cluster,
};
use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

const PLUGIN_TOPOLOGY_FILENAME: &str = "topology.toml";

fn plugin_dylib_filename() -> String {
    let plugin_root_dir = plugin_root_dir();
    let package_name = plugin_root_dir.file_name().and_then(OsStr::to_str).unwrap();
    format!("lib{}.{LIB_EXT}", package_name.replace('-', "_"))
}

/// Constructs a path to the shared library of the plugin
/// located by passed `plugin_path`.
pub fn plugin_dylib_path(plugin_path: &Path) -> PathBuf {
    plugin_path
        .join("target")
        .join("debug")
        .join(plugin_dylib_filename())
}

/// Constructs a path to the topology file of the plugin.
pub fn plugin_topology_path(plugin_path: &Path) -> PathBuf {
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
///                         contain test function symbol.
///
pub fn lua_ffi_call_unit_test(test_fn_name: &str, plugin_dylib_path: &str) -> String {
    format!(
        r#"\lua
"[*] Running unit-test '{test_fn_name}'"

ffi = require("ffi")
ffi.cdef[[void {test_fn_name}();]]
dylib = "{plugin_dylib_path}"
ffi.load(dylib).{test_fn_name}()

"[*] Test '{test_fn_name}' has been finished"
true"#
    )
}

pub fn verify_unit_test_output(output: &str) -> anyhow::Result<()> {
    if output.contains("cannot open shared object file") {
        bail!("failed to open plugin shared library")
    } else if output.contains("missing declaration") || output.contains("undefined symbol") {
        bail!("failed to call unit-test routine: missing symbol in plugin shared library")
    } else if !output.contains("true") {
        bail!("test has finished unexpectedly")
    }

    Ok(())
}

/// Creates new instance of Picodata [`Cluster`].
///
/// ### Arguments
/// - `plugin_path` - path to the plugin root directory.
///                   If `None`, directory is identified automatically.
/// - `plugin_topology` - instance of `PluginTopology`.
///                   If `None`, topology is parsed from default path.
/// - `timeout` - timeout after cluster start.
///
pub fn create_cluster(
    plugin_path: Option<PathBuf>,
    plugin_topology: Option<PluginTopology>,
    timout: Duration,
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
    Cluster::new(plugin_path, plugin_topology.unwrap(), timout)
        .expect("Failed to create the cluster")
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
        let plugin_topology_path = plugin_topology_path(&plugin_root_dir());
        let plugin_topology = parse_topology(&plugin_topology_path).unwrap();

        SingleNodeTopologyTransformer::transform(&plugin_topology)
    })
}
