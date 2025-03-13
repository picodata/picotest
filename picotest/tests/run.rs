mod helpers;

use helpers::{plugin, TestPlugin};
use picotest::*;
use rstest::*;
use std::collections::HashMap;

#[picotest(path = "../tmp/test_plugin")]
fn test_func_install_plugin(plugin: &TestPlugin) {
    let enabled = cluster.run_query(format!(
        r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
        plugin.name
    ));
    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
}

#[picotest(path = "../tmp/test_plugin")]
fn test_apply_config(plugin: &TestPlugin) {
    let must_be_overriden = "should_be_overridden_after_apply";
    let service_config = HashMap::from([(
        "value".to_string(),
        serde_yaml::to_value(must_be_overriden).unwrap(),
    )]);
    let plugin_config = HashMap::from([(plugin.service_name.clone(), service_config)]);

    cluster
        .apply_config(plugin_config)
        .expect("Failed to apply test plugin configuration");

    let service_properties = cluster
        .run_query(format!(
            r#"SELECT key, value FROM _pico_plugin_config 
                    WHERE plugin = '{}' AND entity = '{}';"#,
            plugin.name, plugin.service_name
        ))
        .expect("Failed to run query");

    // TODO: more fine grained verification of key-value pair.
    assert!(service_properties.contains(must_be_overriden));
}

#[picotest(path = "../tmp/test_plugin")]
mod test_mod {
    use crate::{plugin, TestPlugin};
    use std::sync::OnceLock;
    use uuid::Uuid;

    static CLUSTER_UUID: OnceLock<Uuid> = OnceLock::new();

    fn test_once_cluster_1(plugin: &TestPlugin) {
        let cluster_uuid = CLUSTER_UUID.get_or_init(|| cluster.uuid);
        assert_eq!(cluster_uuid, &cluster.uuid);

        let enabled = cluster.run_query(format!(
            r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
            plugin.name
        ));
        assert!(enabled.is_ok());
        assert!(enabled.is_ok_and(|enabled| enabled.contains("true")))
    }

    fn test_once_cluster_2(plugin: &TestPlugin) {
        let cluster_uuid = CLUSTER_UUID.get_or_init(|| cluster.uuid);
        assert_eq!(cluster_uuid, &cluster.uuid);

        let enabled = cluster.run_query(format!(
            r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
            plugin.name
        ));
        assert!(enabled.is_ok());
        assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
    }
}

mod picotest_unit_macro {

    use crate::helpers::{self, fresh_plugin, TestPlugin};
    use constcat::concat;
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
}
