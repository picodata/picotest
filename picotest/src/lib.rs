pub use picotest_helpers::{
    topology::PluginTopology, Cluster, PICOTEST_USER, PICOTEST_USER_PASSWORD,
};
pub use picotest_macros::*;
pub use rstest::*;
pub use std::{panic, path::PathBuf, sync::OnceLock, time::Duration};

pub mod internal;
pub mod runner;

pub static SESSION_CLUSTER: OnceLock<Cluster> = OnceLock::new();

pub type PluginConfigMap = picotest_helpers::PluginConfigMap;

#[fixture]
pub fn cluster(
    #[default(None)] plugin_path: Option<&str>,
    #[default(5)] timeout_secs: u64,
) -> &'static Cluster {
    get_or_create_session_cluster(plugin_path, None, timeout_secs)
}

pub fn get_or_create_session_cluster(
    plugin_path: Option<&str>,
    plugin_topology: Option<&PluginTopology>,
    timeout_secs: u64,
) -> &'static Cluster {
    SESSION_CLUSTER.get_or_init(|| {
        let plugin_path = plugin_path.map(PathBuf::from);
        let plugin_topology = plugin_topology.cloned();
        let timeout = Duration::from_secs(timeout_secs);

        internal::create_cluster(plugin_path, plugin_topology, timeout)
    })
}

#[ctor::dtor]
unsafe fn tear_down() {
    SESSION_CLUSTER.get().map(|cls| cls.stop());
}
