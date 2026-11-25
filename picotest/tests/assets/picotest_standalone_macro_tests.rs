//! Tests for #\[picotest_unit\] macro in a crate that is not a plugin.
//!
//! They are expected to be executed inside a library crate
//! without `topology.toml` and plugin manifest.

pub mod should_success {
    #[picotest::picotest_unit]
    fn a_test_should_success() {
        println!("Hello from test_should_success");
    }

    #[picotest::picotest_unit]
    fn b_test_should_success() {
        println!("Hello from test_should_success");
    }
}

pub mod should_fail {
    #[picotest::picotest_unit]
    fn test_should_fail() {
        panic!("Hello from test_should_fail");
    }
}
