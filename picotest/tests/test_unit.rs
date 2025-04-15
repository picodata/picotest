mod helpers;

use constcat::concat;
use helpers::{fresh_plugin, TestPlugin};
use rstest::rstest;
use std::io::Write;
use std::process::ExitStatus;
use std::time::Duration;
use std::{fs, path::PathBuf};

const TEST_SOURCE_MODULE_NAME: &str = "picotest_unit_macro_tests";
const TEST_SOURCE_FILE_PATH: &str = concat!("./tests/assets/", TEST_SOURCE_MODULE_NAME, ".rs");
const TESTS_EXECUTION_TIMELIMIT: Duration = Duration::from_secs(1200);

fn run_cargo_test(plugin_path: &PathBuf, module_name: &str) -> (ExitStatus, String) {
    helpers::run_cargo_test(
        plugin_path,
        &["--test", module_name, "--nocapture", "--test-threads=1"],
        TESTS_EXECUTION_TIMELIMIT,
    )
}

// Run tests that's supposed to finish with success.
fn assert_success_tests(plugin_path: &PathBuf) {
    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_success");
    let (exit_status, stdout) = run_cargo_test(plugin_path, module_name);

    assert!(
        exit_status.success(),
        "tests are supposed to finish successfully"
    );
    assert!(stdout.contains("Hello from test_should_success"));
}

// Run tests that's supposed to finish with failure.
fn assert_failed_tests(plugin_path: &PathBuf) {
    let module_name = concat!(TEST_SOURCE_MODULE_NAME, "::should_fail");
    let (exit_status, stdout) = run_cargo_test(plugin_path, module_name);

    assert!(
        !exit_status.success(),
        "tests are supposed to finish with failure"
    );
    assert!(stdout.contains("Hello from test_should_fail"));
}

#[rstest]
fn tests(fresh_plugin: &TestPlugin) {
    let plugin_sources = fresh_plugin.path.join("src");
    let test_source_path = PathBuf::from(TEST_SOURCE_FILE_PATH);
    let test_source_filename = test_source_path.file_name().unwrap();

    // Copy *.rs source file with tests to plugin directory.
    fs::copy(&test_source_path, plugin_sources.join(test_source_filename))
        .expect("Failed to copy test file to plugin directory");

    // Add test module to test plugin library.
    // This is necessary to run tests using "cargo test".
    {
        let mut lib_rs = fs::OpenOptions::new()
            .append(true)
            .open(plugin_sources.join("lib.rs"))
            .expect("Failed to open plugin lib.rs");

        writeln!(lib_rs, "\nmod {TEST_SOURCE_MODULE_NAME};")
            .expect("Failed to add test module to lib.rs");
    }

    assert_success_tests(&fresh_plugin.path);
    assert_failed_tests(&fresh_plugin.path);
}
