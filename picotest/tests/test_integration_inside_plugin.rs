mod helpers;

use constcat::concat;
use helpers::{
    add_source_file_to_plugin, fresh_plugin, run_cargo_test_in_plugin_workspace, LineMatcher,
    TestPlugin,
};
use rstest::rstest;

const TEST_SOURCE_MODULE_NAME: &str = "picotest_macro_tests";
const TEST_SOURCE_FILE_PATH: &str = concat!(TEST_SOURCE_MODULE_NAME, ".rs");

#[rstest]
fn run_integration_tests_inside_plugin_workspace(fresh_plugin: &TestPlugin) {
    add_source_file_to_plugin(fresh_plugin, asset!(TEST_SOURCE_FILE_PATH).into());
    let mut line_matcher = LineMatcher::new("Hello from integration_test_inside_plugin");
    let exit_status = run_cargo_test_in_plugin_workspace(
        &fresh_plugin.path,
        TEST_SOURCE_MODULE_NAME,
        &mut line_matcher,
    );

    assert!(
        exit_status.success(),
        "tests are supposed to finish successfully"
    );
    assert!(line_matcher.has_matched());
}
