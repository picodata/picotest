pub use std::{sync::OnceLock, time::Duration};

use internal::plugin_root_dir;
pub use picotest_helpers::Cluster;
pub use picotest_macros::*;
pub mod internal;
pub use rstest::*;
pub use std::panic;

pub static SESSION_CLUSTER: OnceLock<Cluster> = OnceLock::new();

#[fixture]
pub fn cluster(
    #[default(None)] plugin_path: Option<&str>,
    #[default(5)] timeout_secs: u64,
) -> &'static Cluster {
    SESSION_CLUSTER.get_or_init(|| {
        let timeout = Duration::from_secs(timeout_secs);
        // Look up plugin root directory automatically
        // unless explicitly specified.
        let plugin_path = match plugin_path {
            None => plugin_root_dir().to_string_lossy().into_owned(),
            Some(path) => path.to_string(),
        };
        Cluster::new(&plugin_path, timeout)
            .expect("Failed to create the cluster")
            .run()
            .expect("Failed to start the cluster")
    })
}

#[ctor::dtor]
unsafe fn tear_down() {
    SESSION_CLUSTER.get().map(|cls| cls.stop());
}
