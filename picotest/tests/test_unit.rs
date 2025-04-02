mod helpers;

use constcat::concat;
use helpers::{
    add_source_file_to_plugin, fresh_plugin, run_cargo_test_in_plugin_workspace, LineMatcher,
    TestPlugin,
};
use rstest::rstest;
use std::path::PathBuf;

const TEST_SOURCE_MODULE_NAME: &str = "picotest_unit_macro_tests";
const TEST_SOURCE_FILE_PATH: &str = concat!(TEST_SOURCE_MODULE_NAME, ".rs");

// Run tests that's supposed to finish with success.
fn assert_success_tests(plugin_path: &PathBuf) {
    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_success");
    let mut line_matcher = LineMatcher::new("Hello from test_should_success");
    let exit_status =
        run_cargo_test_in_plugin_workspace(plugin_path, module_name, &mut line_matcher);

    assert!(
        exit_status.success(),
        "tests are supposed to finish successfully"
    );
    assert!(line_matcher.has_matched());
}

// Run tests that's supposed to finish with failure.
fn assert_failed_tests(plugin_path: &PathBuf) {
    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_fail");
    let mut line_matcher = LineMatcher::new("Hello from test_should_fail");
    let exit_status =
        run_cargo_test_in_plugin_workspace(plugin_path, module_name, &mut line_matcher);

    assert!(
        !exit_status.success(),
        "tests are supposed to finish with failure"
    );
    assert!(line_matcher.has_matched());
}

#[rstest]
fn run_unit_tests_inside_plugin_workspace(fresh_plugin: &TestPlugin) {
    add_source_file_to_plugin(fresh_plugin, asset!(TEST_SOURCE_FILE_PATH).into());

    assert_success_tests(&fresh_plugin.path);
    assert_failed_tests(&fresh_plugin.path);
}
