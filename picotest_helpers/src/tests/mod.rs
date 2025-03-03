use crate::{
    apply_plugin_configuration, replace_services_configuration, MANIFEST_SERVICES,
    MANIFEST_SERVICE_NAME,
};
use rstest::rstest;
use std::{collections::HashMap, path::PathBuf};
use tempfile::tempdir;

#[rstest]
// Valid case. Plugin config has all services
// listed in manifest file. All of them should
// replace manifest default configuration.
#[case(
    "
        service1:
            value1: overriden1
        service2:
            value1: overriden2
            value2: 22
    ",
    "
        services:
          - name: service1
            default_configuration:
                value1: something1
          - name: service2
            default_configuration:
                value1: something2
                value2: 0
    "
)]
// Plugin config has one extra service.
// It should be ignored during replacement.
#[case(
    "
        service1:
            value1: overriden1
        service2:
            value1: overriden2
            value2: 22
    ",
    "
        services:
          - name: service1
            default_configuration:
                value1: something1
    "
)]
// Manifest has one extra service that's not
// listed in plugin configuration.
// It should not have been altered.
#[case(
    "
        service1:
            value1: overriden1
    ",
    "
        services:
          - name: service1
            default_configuration:
                value1: something1
          - name: service2
            default_configuration:
                value1: something2
                value2: 0
    "
)]
// Input configuration is empty.
// Output manifest should not be changed.
#[case(
    "",
    "
        services:
          - name: service1
            default_configuration:
                value1: something1
    "
)]
fn test_replace_services_configuration(#[case] plugin_config: &str, #[case] plugin_manifest: &str) {
    let plugin_config: serde_yaml::Value =
        serde_yaml::from_str(&plugin_config).expect("Failed to parse test case config");
    let input_manifest: serde_yaml::Value =
        serde_yaml::from_str(&plugin_manifest).expect("Failed to parse test case manifest");

    let mut output_manifest = input_manifest.clone();
    replace_services_configuration(&plugin_config, &mut output_manifest)
        .expect("Failed to replace configuration of service(s)");

    let get_manifest_services = |manifest: &serde_yaml::Value| {
        manifest
            .get(MANIFEST_SERVICES)
            .expect("Failed to extract services mapping from plugin manifest")
            .as_sequence()
            .unwrap()
            .clone()
    };

    let get_manifest_service_name = |service: &serde_yaml::Value| {
        service
            .get(MANIFEST_SERVICE_NAME)
            .expect("Failed to extract name of the service from plugin manifest")
            .as_str()
            .unwrap()
            .to_owned()
    };

    let input_manifest_services = get_manifest_services(&input_manifest);
    let output_manifest_services = get_manifest_services(&output_manifest);

    assert_eq!(
        input_manifest_services.iter().count(),
        output_manifest_services.iter().count(),
        "number of services should not have changed after replacement"
    );

    let mut not_changed_services: HashMap<String, serde_yaml::Value> = HashMap::new();
    for service in output_manifest_services.iter() {
        let service_name = get_manifest_service_name(service);

        // Both configs should be equal after replacement.
        let manifest_service_config = service.get("default_configuration").unwrap();
        let Some(plugin_service_config) = plugin_config.get(&service_name) else {
            // Input plugin config doesn't have service found in the manifest. It's ok.
            // Check later that these configurations has not changed.
            not_changed_services.insert(service_name, service.clone());
            continue;
        };

        assert_eq!(manifest_service_config, plugin_service_config);
    }

    // Verify that configs that's not supposed to be changed
    // (i.e. missing in input plugin config) was not changed during replacement.
    for service in input_manifest_services.iter() {
        let service_name = get_manifest_service_name(service);

        if let Some(not_changed_service) = not_changed_services.get(&service_name) {
            assert_eq!(
                service, not_changed_service,
                "missing plugin service configurations should not change final manifest"
            );
        }
    }
}

#[rstest]
#[case(
    "./src/tests/resources/test_custom_plugin_config/plugin_config.yaml",
    "./src/tests/resources/test_custom_plugin_config/input_manifest.yaml",
    "./src/tests/resources/test_custom_plugin_config/output_manifest.yaml"
)]
fn test_apply_plugin_configuration(
    #[case] plugin_config_path: PathBuf,
    #[case] input_manifest_path: PathBuf,
    #[case] output_manifest_path: PathBuf,
) {
    // Copy input manifest to the temp. file, because replacement is done
    // in-place.
    let tmp_dir = tempdir().expect("Failed to create temp directory");
    let tmp_manifest = tmp_dir.path().join("manifest.yaml");

    std::fs::copy(input_manifest_path, &tmp_manifest)
        .expect("Failed to copy input manifest to temp location");

    apply_plugin_configuration(plugin_config_path, &tmp_manifest)
        .expect("Failed to apply plugin configuration");

    // Verify replacement.
    let actual = std::fs::read_to_string(tmp_manifest).unwrap();
    let reference = std::fs::read_to_string(output_manifest_path).unwrap();

    assert_eq!(reference, actual);
}
