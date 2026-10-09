use picotest::internal::create_cluster;

#[test]
fn test_stop_and_restart_instance() {
    let cluster = create_cluster(None);
    let instance = &cluster.instances()[1];
    assert!(instance.is_running());

    cluster.stop_instance(instance).unwrap();
    assert!(!instance.is_running());

    cluster.start_instance(instance).unwrap();
    assert!(instance.is_running());
    assert!(instance.run_lua("return 1 + 1").unwrap().contains("2"));

    cluster.restart_instance(instance).unwrap();
    assert!(instance.is_running());
    assert!(instance.run_lua("return 1 + 1").unwrap().contains("2"));
}

#[test]
fn test_expel_instance() {
    let cluster = create_cluster(None);
    let instance = &cluster.instances()[3];
    assert!(instance.is_running());

    // Running instance can only be expelled forcibly.
    assert!(cluster.expel_instance(instance, false).is_err());
    assert!(instance.is_running());

    cluster.expel_instance(instance, true).unwrap();
    assert!(!instance.is_running());

    let state = cluster
        .run_query(format!(
            "SELECT current_state FROM _pico_instance WHERE name = '{}';",
            instance.instance_name
        ))
        .unwrap();
    assert!(state.contains("Expelled"), "state: {state}");
}
