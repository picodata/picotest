mod common;

use common::{
    add_source_file_to_crate, create_test_crate, run_cargo_test_in_plugin_workspace, LineMatcher,
};
use constcat::concat;

const TEST_SOURCE_MODULE_NAME: &str = "picotest_standalone_macro_tests";
const TEST_SOURCE_FILE_PATH: &str = concat!(TEST_SOURCE_MODULE_NAME, ".rs");

/// `#[picotest_unit]` in a crate that is not a plugin runs its tests
/// on a single picodata instance without plugins.
#[test]
fn run_unit_tests_inside_standalone_crate() {
    let test_crate = create_test_crate();
    add_source_file_to_crate(&test_crate, asset!(TEST_SOURCE_FILE_PATH).into());

    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_success");
    let mut line_matcher = LineMatcher::new("Hello from test_should_success");
    let exit_status =
        run_cargo_test_in_plugin_workspace(&test_crate.path, module_name, &mut line_matcher);
    assert!(
        exit_status.success(),
        "tests are supposed to finish successfully"
    );
    assert!(line_matcher.has_matched());

    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_fail");
    let mut line_matcher = LineMatcher::new("Hello from test_should_fail");
    let exit_status =
        run_cargo_test_in_plugin_workspace(&test_crate.path, module_name, &mut line_matcher);
    assert!(
        !exit_status.success(),
        "tests are supposed to finish with failure"
    );
    assert!(line_matcher.has_matched());
}
