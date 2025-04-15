pub use std::{sync::OnceLock, time::Duration};

pub use picotest_helpers::Cluster;
pub use picotest_macros::*;
pub mod internal;
pub use rstest::*;
pub use std::panic;

pub static SESSION_CLUSTER: OnceLock<Cluster> = OnceLock::new();

#[fixture]
pub fn cluster(#[default(".")] plugin_path: &str, #[default(5)] timeout: u64) -> &'static Cluster {
    SESSION_CLUSTER.get_or_init(|| {
        let timeout = Duration::from_secs(timeout);
        Cluster::new(plugin_path, timeout)
            .expect("Failed to parse topology")
            .run()
            .expect("Failed to start the cluster")
    })
}

#[ctor::dtor]
unsafe fn tear_down() {
    SESSION_CLUSTER.get().map(|cls| cls.stop());
}
