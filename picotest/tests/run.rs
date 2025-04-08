mod helpers;

macro_rules! asset {
    ($filename:expr) => {
        concat!("./tests/assets/", $filename)
    };
}

mod picotest_macro {

    use crate::helpers::{
        add_source_file_to_plugin, fresh_plugin, plugin, run_cargo_test_in_plugin_workspace,
        LineMatcher, TestPlugin,
    };
    use constcat::concat;
    use picotest::*;
    use std::collections::HashMap;

    const TEST_SOURCE_MODULE_NAME: &str = "picotest_macro_tests";
    const TEST_SOURCE_FILE_PATH: &str = concat!(TEST_SOURCE_MODULE_NAME, ".rs");

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
        use crate::helpers::{plugin, TestPlugin};
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

        #[case(1, 1)]
        #[case(2, 2)]
        fn test_with_once(_plugin: &TestPlugin, #[case] input: u32, #[case] output: u32) {
            assert_eq!(input, output)
        }
    }

    #[rstest]
    fn tests_inside_plugin_workspace(fresh_plugin: &TestPlugin) {
        add_source_file_to_plugin(fresh_plugin, asset!(TEST_SOURCE_FILE_PATH).into());
        let mut line_matcher = LineMatcher::new("Hello from test_func_install_plugin");
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
}

mod picotest_unit_macro {

    use crate::helpers::{
        add_source_file_to_plugin, fresh_plugin, run_cargo_test_in_plugin_workspace, LineMatcher,
        TestPlugin,
    };
    use constcat::concat;
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
    fn tests_inside_plugin_workspace(fresh_plugin: &TestPlugin) {
        add_source_file_to_plugin(fresh_plugin, asset!(TEST_SOURCE_FILE_PATH).into());

        assert_success_tests(&fresh_plugin.path);
        assert_failed_tests(&fresh_plugin.path);
    }
}
