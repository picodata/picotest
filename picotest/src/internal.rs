//! Picotest Internal API
//!
//! Contains helper routines called by proc macro unfolding.
//! This module isn't supposed to be used manually.

use anyhow::bail;
use std::{
    env,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

fn plugin_dylib_filename() -> String {
    let package_name = env::var("CARGO_PKG_NAME").unwrap();
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
\set delimiter !
"[*] Running unit-test '{test_fn_name}'"!

local ffi = require("ffi")
ffi.cdef[[void {test_fn_name}();]]
local dylib = "{plugin_dylib_path}"
ffi.load(dylib).{test_fn_name}()!

"[*] Test '{test_fn_name}' has been finished"!
true!"#
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
