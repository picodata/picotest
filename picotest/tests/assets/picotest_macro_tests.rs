//! Tests for #\[picotest\] macro
//!
//! They are expected to be executed inside plugin
//! workspace.
//!

use rstest::rstest;
use picotest::*;

#[picotest]
fn test_integration_test_inside_plugin() {
    println!("Hello from integration_test_inside_plugin");
}
