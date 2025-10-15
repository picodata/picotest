//! Tests for #\[picotest_unit\] macro
//!
//! They are expected to be executed inside plugin
//! package.
//!
//! Note: this module should be embedded in the plugin
//! created by pike, because #\[picotest_unit\] works only
//! inside pike plugins.
//!
//!

pub mod should_success {
    #[picotest::picotest_unit]
    fn test_should_success() {
        println!("Hello from test_should_success");
    }

    #[should_panic]
    #[picotest::picotest_unit]
    fn test_should_success_but_panic() {
        assert!(false);
    }
}

pub mod should_fail {
    #[picotest::picotest_unit]
    fn test_should_fail() {
        panic!("Hello from test_should_fail");
    }
}
