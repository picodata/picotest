#[path = "../common/mod.rs"]
mod common;
mod integration;
mod pgproto;

use ctor::ctor;

pub use common::{plugin, TestPlugin};

#[ctor]
unsafe fn init_plugin() {
    plugin();
}
