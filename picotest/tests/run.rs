mod helpers;

use constcat::concat;
use helpers::{build_plugin, wait_for_proc};
use picotest::*;
use rstest::*;
use std::{collections::HashMap, fs, time::Duration};

pub const TMP_DIR: &str = "../tmp/";
pub const PLUGIN_NAME: &str = "test_plugin";
pub const PLUGIN_DIR: &str = concat!(TMP_DIR, PLUGIN_NAME);
pub const PLUGIN_SERVICE_NAME: &str = "main";

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

#[picotest(path = "../tmp/test_plugin")]
fn test_func_install_plugin(plugin: &Plugin) {
    let enabled = cluster.run_query(format!(
        r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
        plugin.name
    ));
    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
}

#[picotest(path = "../tmp/test_plugin")]
fn test_apply_config(plugin: &Plugin) {
    let must_be_overriden = "should_be_overridden_after_apply";
    let service_config = HashMap::from([(
        "value".to_string(),
        serde_yaml::to_value(must_be_overriden).unwrap(),
    )]);
    let plugin_config = HashMap::from([(PLUGIN_SERVICE_NAME.to_string(), service_config)]);

    cluster
        .apply_config(plugin_config)
        .expect("Failed to apply test plugin configuration");

    let service_properties = cluster
        .run_query(format!(
            r#"SELECT key, value FROM _pico_plugin_config 
                    WHERE plugin = '{}' AND entity = '{}';"#,
            plugin.name, PLUGIN_SERVICE_NAME
        ))
        .expect("Failed to run query");

    // TODO: more fine grained verification of key-value pair.
    assert!(service_properties.contains(must_be_overriden));
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
