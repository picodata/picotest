use dtor::dtor;
pub use picotest_helpers::{
    topology::PluginTopology, Cluster, PICOTEST_USER, PICOTEST_USER_PASSWORD,
};
pub use picotest_macros::*;
pub use rstest::*;
pub use std::{panic, path::PathBuf, sync::OnceLock, time::Duration};

pub mod internal;

pub static SESSION_CLUSTER: OnceLock<Cluster> = OnceLock::new();

pub type PluginConfigMap = picotest_helpers::PluginConfigMap;

#[fixture]
pub fn cluster() -> &'static Cluster {
    get_or_create_session_cluster(None)
}

pub fn get_or_create_session_cluster(plugin_topology: Option<&PluginTopology>) -> &'static Cluster {
    SESSION_CLUSTER.get_or_init(|| {
        env_logger::init();
        let plugin_topology = plugin_topology.cloned();

        internal::create_cluster(plugin_topology)
    })
}

#[dtor]
unsafe fn tear_down() {
    if let Some(cluster) = SESSION_CLUSTER.get() {
        cluster.stop().expect("Failed to stop the cluster");
    }
}
