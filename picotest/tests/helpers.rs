#![allow(dead_code)]
use constcat::concat;
use picotest_helpers::run_pike;
use rstest::fixture;
use std::fs;
use std::io::{BufRead, BufReader, Error};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const TMP_DIR: &str = "../tmp/";
const PLUGIN_NAME: &str = "test_plugin";
const PLUGIN_DIR: &str = concat!(TMP_DIR, PLUGIN_NAME);
const PLUGIN_SERVICE_NAME: &str = "main";
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Create new or return existing test plugin instance.
#[fixture]
#[once]
pub fn plugin() -> TestPlugin {
    create_test_plugin(false)
}

/// Create fresh test plugin instance.
///
/// Note: this fixture always overrides
/// test plugin directory.
#[fixture]
#[once]
pub fn fresh_plugin() -> TestPlugin {
    create_test_plugin(true)
}

#[derive(Debug)]
pub struct TestPlugin {
    pub name: String,
    pub path: PathBuf,
    pub service_name: String,
}

pub fn create_test_plugin(remove_if_exists: bool) -> TestPlugin {
    if remove_if_exists && fs::metadata(PLUGIN_DIR).is_ok() {
        fs::remove_dir_all(PLUGIN_DIR).expect("Failed to remove test plugin directory");
    }

    fs::create_dir_all(TMP_DIR).expect("Failed to create directory for pike plugin");

    let _ = wait_for_process_termination(
        run_pike(vec!["plugin", "new", PLUGIN_NAME], TMP_DIR)
            .expect("Failed to generate plugin boilerplate code"),
        PROCESS_WAIT_TIMEOUT,
    );

    assert!(fs::metadata(concat!(PLUGIN_DIR, "/Cargo.toml")).is_ok());
    assert!(fs::metadata(concat!(PLUGIN_DIR, "/topology.toml")).is_ok());

    // Add picotest to the test plugin dependencies.
    // This is mandatory for running tests of #[picotest_unit] macro.
    {
        let process = add_package_to_test_plugin(env!("CARGO_MANIFEST_DIR"), PLUGIN_DIR)
            .expect("Failed to add picotest to test plugin dependencies");
        let exit_status = wait_for_process_termination(process, PROCESS_WAIT_TIMEOUT);
        assert!(exit_status.success());
    }

    TestPlugin {
        name: PLUGIN_NAME.parse().unwrap(),
        path: PathBuf::from(PLUGIN_DIR),
        service_name: PLUGIN_SERVICE_NAME.parse().unwrap(),
    }
}

/// Run tests by executing "cargo test".
///
/// ### Arguments
/// - `manifest_dir` - the directory containing the manifest of package under test.
/// - `test_args` - array of args passed to "cargo test" command after '--'.
/// - `timeout` - test execution time limit.
///
/// ### Returns
/// Exit status and stdout of finished "cargo test" subprocess.
///
pub fn run_cargo_test(
    manifest_dir: &PathBuf,
    test_args: &[&str],
    timeout: Duration,
) -> (ExitStatus, String) {
    println!(
        "\nRunning \"cargo test\" in '{}' with options {:?}. Allowed execution time is {}s",
        manifest_dir.display(),
        test_args,
        timeout.as_secs()
    );

    let mut child = Command::new("cargo")
        .arg("test")
        .arg("--quiet")
        .arg("--")
        .args(test_args)
        .current_dir(manifest_dir)
        .stdout(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn \"cargo test\" process");

    let stdout = child
        .stdout
        .take()
        .expect("Failed to obtain stdout handle of testing process");

    let exit_status = wait_for_process_termination(child, timeout);
    if !exit_status.success() {
        println!(
            "\"cargo test\" in '{}' has finished with failure",
            manifest_dir.display(),
        );
    } else {
        println!(
            "\"cargo test\" in '{}' has finished successfully",
            manifest_dir.display(),
        );
    }

    let stdout = BufReader::new(stdout).lines().map(Result::unwrap).collect();

    (exit_status, stdout)
}

fn wait_for_process_termination(mut child: Child, timeout: Duration) -> ExitStatus {
    let start_time = Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            panic!(
                "Process running for too long. Allowed execution time is {}s.",
                timeout.as_secs(),
            );
        }
        match child.try_wait().unwrap() {
            Some(exit_status) => return exit_status,
            None => {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Adds package to test plugin dependencies through "cargo add" command.
///
/// ### Arguments
///     - `manifest_dir` - the directory containing the manifest of adding package.
///     - `test_plugin` - descriptor of test plugin.
///
fn add_package_to_test_plugin(manifest_dir: &str, plugin_path: &str) -> Result<Child, Error> {
    Command::new("cargo")
        .arg("add")
        .arg("--quiet")
        .arg("--path")
        .arg(manifest_dir)
        .current_dir(plugin_path)
        .spawn()
}
