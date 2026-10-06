use dtor::dtor;
pub use picotest_helpers::{
    topology::PluginTopology, Cluster, PICOTEST_USER, PICOTEST_USER_PASSWORD,
};
pub use picotest_macros::*;
pub use rstest::*;
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
pub use std::{panic, path::PathBuf, sync::OnceLock, time::Duration};

pub mod internal;

type ClusterKey = (PathBuf, String);

static SESSION_CLUSTERS: Mutex<BTreeMap<ClusterKey, &'static OnceLock<Cluster>>> =
    Mutex::new(BTreeMap::new());

pub type PluginConfigMap = picotest_helpers::PluginConfigMap;

#[fixture]
pub fn cluster(#[default(None)] plugin_path: Option<&str>) -> &'static Cluster {
    get_or_create_session_cluster(plugin_path, None)
}

#[fixture]
pub fn cluster_with_topology(
    #[default(None)] plugin_path: Option<&str>,
    #[default(None)] topology_path: Option<&str>,
) -> &'static Cluster {
    let Some(topology_path) = topology_path else {
        return get_or_create_session_cluster(plugin_path, None);
    };

    let plugin_root = plugin_path.map_or_else(internal::plugin_root_dir, PathBuf::from);
    let topology_path = plugin_root.join(topology_path);
    let topology = picotest_helpers::topology::parse_topology(&topology_path)
        .unwrap_or_else(|err| panic!("Failed to load cluster topology: {err:#}"));

    get_or_create_session_cluster(plugin_path, Some(&topology))
}

pub fn get_or_create_session_cluster(
    plugin_path: Option<&str>,
    plugin_topology: Option<&PluginTopology>,
) -> &'static Cluster {
    let plugin_path = plugin_path.map_or_else(internal::plugin_root_dir, PathBuf::from);
    let plugin_topology = plugin_topology.map_or_else(
        || internal::default_plugin_topology(&plugin_path),
        Clone::clone,
    );

    let key = (plugin_path.clone(), format!("{plugin_topology:?}"));
    let cell = *SESSION_CLUSTERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key)
        .or_insert_with(|| Box::leak(Box::default()));

    cell.get_or_init(|| {
        let _ = env_logger::try_init();

        internal::create_cluster(Some(plugin_path), Some(plugin_topology))
    })
}

#[dtor]
unsafe fn tear_down() {
    let clusters = SESSION_CLUSTERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    let mut failed = false;
    for cluster in clusters.values().filter_map(|cell| cell.get()) {
        if let Err(err) = cluster.stop() {
            eprintln!("Failed to stop the cluster: {err:#}");
            failed = true;
        }
    }
    assert!(!failed, "Failed to stop the cluster");
}
