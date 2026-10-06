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
pub fn cluster() -> &'static Cluster {
    get_or_create_session_cluster(None)
}

/// Path to the cluster topology file.
#[derive(Clone, Debug, Default)]
pub struct TopologyPath(Option<PathBuf>);

impl TopologyPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(Some(path.into()))
    }

    fn load(&self) -> PluginTopology {
        let plugin_root = internal::plugin_root_dir();
        let path = match &self.0 {
            Some(path) => plugin_root.join(path),
            None => internal::plugin_topology_path(&plugin_root),
        };

        picotest_helpers::topology::parse_topology(&path)
            .unwrap_or_else(|err| panic!("Failed to load cluster topology: {err:#}"))
    }
}

impl From<&str> for TopologyPath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<PathBuf> for TopologyPath {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

#[fixture]
pub fn cluster_with_topology(
    #[default(TopologyPath::default())] topology: TopologyPath,
) -> &'static Cluster {
    get_or_create_session_cluster(Some(&topology.load()))
}

pub fn get_or_create_session_cluster(plugin_topology: Option<&PluginTopology>) -> &'static Cluster {
    let plugin_topology =
        plugin_topology.map_or_else(|| TopologyPath::default().load(), Clone::clone);

    let key = (internal::plugin_root_dir(), format!("{plugin_topology:?}"));
    let cell = *SESSION_CLUSTERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key)
        .or_insert_with(|| Box::leak(Box::default()));

    cell.get_or_init(|| {
        let _ = env_logger::try_init();

        internal::create_cluster(Some(plugin_topology))
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
