mod helpers;

use constcat::concat;
use helpers::{build_plugin, wait_for_proc};
use picotest::*;
use rstest::*;
use std::{fs, path::PathBuf, time::Duration};

pub const TMP_DIR: &str = "../tmp/";
pub const PLUGIN_NAME: &str = "test_plugin";
pub const PLUGIN_DIR: &str = concat!(TMP_DIR, PLUGIN_NAME);
pub const PLUGIN_VERSION: &str = "0.1.0";
pub const PLUGIN_SERVICE_NAME: &str = "main";
pub const PLUGIN_MANIFEST_FILENAME: &str = "manifest.yaml";
pub const PLUGIN_MANIFEST_FILENAME_BACKUP: &str = "manifest.backup.yaml";
pub const PLUGIN_MANIFEST_DIR: &str = concat!(
    PLUGIN_DIR,
    "/target/debug/",
    PLUGIN_NAME,
    "/",
    PLUGIN_VERSION
);

#[derive(Debug)]
struct Plugin {
    name: String,
}

#[fixture]
#[once]
pub fn plugin() -> Plugin {
    fs::create_dir_all(TMP_DIR).expect("Failed to create tmp directory");
    let mut proc = run_pike(vec!["plugin", "new", PLUGIN_NAME], TMP_DIR)
        .expect("Failed to generate plugin boilerplate code");
    wait_for_proc(&mut proc, Duration::from_secs(10));
    let _ = build_plugin(PLUGIN_DIR).expect("Failed to build plugin");
    Plugin {
        name: PLUGIN_NAME.to_string(),
    }
}

#[fixture]
pub fn manifest_path() -> PathBuf {
    PathBuf::from(PLUGIN_MANIFEST_DIR).join(PLUGIN_MANIFEST_FILENAME)
}

#[fixture]
pub fn manifest_backup_path() -> PathBuf {
    PathBuf::from(PLUGIN_MANIFEST_DIR).join(PLUGIN_MANIFEST_FILENAME_BACKUP)
}

#[picotest(path = "../tmp/test_plugin")]
fn test_func_install_plugin(plugin: &Plugin) {
    let enabled = cluster.run_query(format!(
        r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
        plugin.name
    ));
    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
}

#[picotest(
    path = "../tmp/test_plugin",
    config_path = "./tests/resources/test_plugin_config.yaml"
)]
fn test_cusom_plugin_configuration(plugin: &Plugin) {
    let service_properties = cluster
        .run_query(format!(
            r#"SELECT key, value FROM _pico_plugin_config WHERE plugin = '{}' AND entity = '{}';"#,
            plugin.name, PLUGIN_SERVICE_NAME
        ))
        .expect("Failed to run query");

    // TODO: more fine grained verification of key-value pair.
    assert!(service_properties.contains("should_be_overridden_in_test"));
}

#[rstest]
fn test_backup_manifest(manifest_path: PathBuf, manifest_backup_path: PathBuf) {
    // Test verifies that replacement of plugin configuration leads to
    // creation of backup manifest file. Original manifest should be
    // restored from backup when cluster is dropped.
    let plugin_path = "../tmp/test_plugin";
    let config_path = "./tests/resources/test_plugin_config.yaml";

    let original_manifest =
        fs::read_to_string(&manifest_path).expect("Failed to read original manifest");
    {
        let _cluster = picotest::run_cluster(plugin_path, Some(config_path), 0);

        assert!(
            fs::metadata(&manifest_backup_path).is_ok(),
            "manifest backup should be created"
        );

        // Cluster is droping. Original manifest should be
        // restored right after cluster is droped.
    }
    let restored_manifest =
        fs::read_to_string(&manifest_path).expect("Failed to read restored manifest");

    assert_eq!(original_manifest, restored_manifest);
    assert!(fs::metadata(manifest_backup_path).is_err());
}

#[picotest(path = "../tmp/test_plugin")]
mod test_mod {
    use crate::{plugin, Plugin};
    use std::sync::OnceLock;
    use uuid::Uuid;

    static CLUSTER_UUID: OnceLock<Uuid> = OnceLock::new();

    fn test_once_cluster_1(plugin: &Plugin) {
        let cluster_uuid = CLUSTER_UUID.get_or_init(|| cluster.uuid);
        assert_eq!(cluster_uuid, &cluster.uuid);

        let enabled = cluster.run_query(format!(
            r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
            plugin.name
        ));
        assert!(enabled.is_ok());
        assert!(enabled.is_ok_and(|enabled| enabled.contains("true")))
    }

    fn test_once_cluster_2(plugin: &Plugin) {
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
