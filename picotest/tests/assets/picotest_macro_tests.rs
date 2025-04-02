//! Tests for #\[picotest\] macro
//!
//! They are expected to be executed inside plugin
//! workspace.
//!

use rstest::rstest;
use picotest::picotest;

#[picotest]
fn test_func_install_plugin() {
    let enabled = cluster.run_query(format!(
        "SELECT enabled FROM _pico_plugin WHERE name = 'test_plugin';"
    ));

    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));

    println!("Hello from test_func_install_plugin");
}
