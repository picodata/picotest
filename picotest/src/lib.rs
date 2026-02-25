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
pub fn cluster(#[default(None)] plugin_path: Option<&str>) -> &'static Cluster {
    get_or_create_session_cluster(plugin_path, None)
}

pub fn get_or_create_session_cluster(
    plugin_path: Option<&str>,
    plugin_topology: Option<&PluginTopology>,
) -> &'static Cluster {
    SESSION_CLUSTER.get_or_init(|| {
        let plugin_path = plugin_path.map(PathBuf::from);
        let plugin_topology = plugin_topology.cloned();

        internal::create_cluster(plugin_path, plugin_topology)
    })
}

#[ctor::dtor]
unsafe fn tear_down() {
    SESSION_CLUSTER.get().map(|cls| cls.stop());
}
