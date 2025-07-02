mod helpers;

use ctor::ctor;
use helpers::{plugin, TestPlugin};
use picotest::*;
use picotest_helpers::{LUA_OUTPUT_FOOTER, LUA_OUTPUT_HEADER};
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
    assert!(!res.contains(LUA_OUTPUT_HEADER));
    assert!(!res.contains(LUA_OUTPUT_FOOTER));
}

#[picotest(path = "../tmp/test_plugin")]
fn test_run_lua_select_and_serialize_output_from_yaml() {
    // This test creates box and insert some values into it.
    // Then tries to select those values through Lua query
    // and deserialize them from YAML.

    let _ = cluster.main()
        .run_lua(
            "
        box.schema.space.create('test_space')
        box.space.test_space:format({
            {name = 's', type = 'string'},
            {name = 'i', type = 'integer'},
            {name = 'a', type = 'array'},
            {name = 'm', type = 'map'}
        })
        box.space.test_space:create_index('i',{parts={2, type = 'integer'}})
        box.space.test_space:insert{ 'aaa', -1, {'list_item1', 'list_item2'}, {Completed={'filepath1.dsv'}}}
        box.space.test_space:insert{ 'bbb', -2, {'list_item1'}, {Completed={'filepath2.dsv'}}}
        box.space.test_space:insert{ 'ccc', -3, {'string with\\nnew\\n\\nlines'}, {Completed={'filepath2.dsv'}}}",
        )
        .expect("Failed to run Lua query");

    let output = cluster
        .main()
        .run_lua("box.space.test_space.index.i:select()")
        .expect("Failed to run Lua query");

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct SomeStruct(String, i32, Vec<String>, SomeStatus);

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    #[serde(untagged)]
    enum SomeStatus {
        Completed(HashMap<String, Vec<String>>),
    }

    let actual: Vec<Vec<SomeStruct>> =
        serde_yaml::from_str(&output).expect("Failed to deserialize struct from YAML string");

    assert_eq!(
        actual,
        vec![vec![
            SomeStruct(
                "ccc".into(),
                -3,
                vec!["string with\nnew\n\nlines".to_string()],
                SomeStatus::Completed([("Completed".into(), vec!["filepath2.dsv".into()])].into())
            ),
            SomeStruct(
                "bbb".into(),
                -2,
                vec!["list_item1".into()],
                SomeStatus::Completed([("Completed".into(), vec!["filepath2.dsv".into()])].into())
            ),
            SomeStruct(
                "aaa".into(),
                -1,
                vec!["list_item1".into(), "list_item2".into()],
                SomeStatus::Completed([("Completed".into(), vec!["filepath1.dsv".into()])].into())
            ),
        ]]
    )
}
