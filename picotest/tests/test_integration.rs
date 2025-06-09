mod helpers;

use ctor::ctor;
use helpers::{plugin, TestPlugin};
use picotest::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};
use uuid::Uuid;

static GLOBAL_CLUSTER_UUID: OnceLock<Uuid> = OnceLock::new();

#[ctor]
fn init_plugin() {
    plugin();
}

#[picotest(path = "../tmp/test_plugin")]
fn test_func_install_plugin(plugin: &TestPlugin) {
    let cluster_uuid = GLOBAL_CLUSTER_UUID.get_or_init(|| cluster.uuid);
    assert_eq!(cluster_uuid, &cluster.uuid);

    let enabled = cluster.run_query(format!(
        r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
        plugin.name
    ));
    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
}

#[picotest(path = "../tmp/test_plugin")]
fn test_apply_config(plugin: &TestPlugin) {
    let cluster_uuid = GLOBAL_CLUSTER_UUID.get_or_init(|| cluster.uuid);
    assert_eq!(cluster_uuid, &cluster.uuid);

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
fn test_get_instances() {
    let cluster_uuid = GLOBAL_CLUSTER_UUID.get_or_init(|| cluster.uuid);
    assert_eq!(cluster_uuid, &cluster.uuid);

    assert_eq!(cluster.instances().len(), 4);
    assert_eq!(cluster.main().pg_port, 5433)
}

#[picotest(path = "../tmp/test_plugin")]
#[case(1, 1)]
#[case(2, 2)]
#[case(3, 3)]
fn test_function_with_case(#[case] input: i32, #[case] expected: i32) {
    let cluster_uuid = GLOBAL_CLUSTER_UUID.get_or_init(|| cluster.uuid);
    assert_eq!(cluster_uuid, &cluster.uuid);
    assert_eq!(input, expected);
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

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleResponse {
    rpc_hello_response: String,
}

#[tokio::test]
#[picotest(path = "../tmp/test_plugin")]
async fn test_rpc_handle(plugin: &TestPlugin) {
    let user_to_send = User {
        name: "Dodo".to_string(),
    };

    let tnt_response = cluster
        .main()
        .execute_rpc::<User, ExampleResponse>(
            &plugin.name,
            "/greetings_rpc",
            &plugin.service_name,
            "0.1.0",
            &user_to_send,
        )
        .await
        .unwrap();

    assert_eq!(
        tnt_response.rpc_hello_response,
        "Hello Dodo, long time no see."
    );
}

#[picotest(path = "../tmp/test_plugin")]
fn test_run_lua_query(_plugin: &TestPlugin) {
    let res = cluster.instances()[1].run_lua("return 1 + 1").unwrap();
    assert!(res.contains("2"));
}
