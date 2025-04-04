#![allow(dead_code)]
use constcat::concat;
use picotest_helpers::run_pike;
use rstest::fixture;
use std::fs;
use std::io::{BufRead, BufReader, Error, Read};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const TMP_DIR: &str = "../tmp/";
const PLUGIN_NAME: &str = "test_plugin";
const PLUGIN_DIR: &str = concat!(TMP_DIR, PLUGIN_NAME);
const PLUGIN_SERVICE_NAME: &str = "main";
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const TESTS_EXECUTION_TIMELIMIT: Duration = Duration::from_secs(1200);

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

/// Processor of logs that finally
/// checks whether line has matched during
/// log line consumption.
///
/// Implementor of `TestOutputConsumer`.
pub struct LineMatcher {
    line: String,
    match_count: u32,
}

impl LineMatcher {
    pub fn new(line: &str) -> Self {
        Self {
            line: line.to_string(),
            match_count: 0,
        }
    }

    /// Returns `true` if line has appeared
    /// in the logs at least once.
    pub fn has_matched(&self) -> bool {
        self.match_count > 0
    }
}

impl TestOutputConsumer for LineMatcher {
    fn consume_line(&mut self, line: &str) {
        println!("{line}");
        if line.contains(&self.line) {
            self.match_count += 1;
        }
    }
}

pub trait TestOutputConsumer {
    /// Consume and process output log line.
    fn consume_line(&mut self, line: &str);
}

/// Executes "cargo test" in plugin workspace.
///
/// ### Arguments
/// * `plugin_root_dir` - root directory of the plugin workspace
/// * `test_filter` - name of the tests passed to `--test` option
/// * `test_output_consumer` - consumer of output logs from running test
///
pub fn run_cargo_test_in_plugin_workspace<T>(
    plugin_root_dir: &PathBuf,
    test_filter: &str,
    test_output_consumer: &mut T,
) -> ExitStatus
where
    T: TestOutputConsumer,
{
    let mut child = run_cargo_test(plugin_root_dir, &["--test", test_filter, "--nocapture"]);

    let stdout = child
        .stdout
        .take()
        .expect("Failed to obtain stdout handle of testing process");

    let mut stderr = child
        .stderr
        .take()
        .expect("Failed to obtain stderr handle of testing process");

    // Observer thread that will monitor running subprocess.
    // On join, returns exit status of "cargo test" even if it was killed
    // due to expired timeout.
    let observer = std::thread::spawn(move || {
        let exit_status = child
            .wait_timeout(TESTS_EXECUTION_TIMELIMIT)
            .expect("Failed to wait for \"cargo test\" termination");

        match exit_status {
            Some(value) => value,
            None => {
                eprintln!("\"cargo tests\" has been running for too long. Killing the process.");
                child.kill().expect("Failed to kill \"cargo tests\"");
                child.wait().expect("Failed to wait killed \"cargo tests\"")
            }
        }
    });

    let reader = BufReader::new(stdout);
    for output_line in reader.lines() {
        let output_line = output_line.expect("Failed to read test output line");
        test_output_consumer.consume_line(&output_line);
    }

    let exit_status = observer.join().expect("Failed to join observer thread");
    if !exit_status.success() {
        eprintln!(
            "\"cargo test\" in '{}' has finished with failure. {}",
            plugin_root_dir.display(),
            exit_status
        );

        let mut stderr_buf = String::new();
        stderr
            .read_to_string(&mut stderr_buf)
            .expect("Failed to read stderr of testing process");

        eprintln!("\n\"cargo test\" stderr:\n\n{stderr_buf}");
    } else {
        println!(
            "\"cargo test\" in '{}' has finished successfully",
            plugin_root_dir.display(),
        );
    }

    exit_status
}

/// Run tests by executing "cargo test".
///
/// ### Arguments
/// - `manifest_dir` - the directory containing the manifest of package under test.
/// - `test_args` - array of args passed to "cargo test" command after '--'.
///
/// ### Returns
/// Instance of [`Child`] describing spawned "cargo test" subprocess.
///
fn run_cargo_test(manifest_dir: &PathBuf, test_args: &[&str]) -> Child {
    println!(
        "\nRunning \"cargo test\" in '{}' with options {:?}",
        manifest_dir.display(),
        test_args,
    );

    Command::new("cargo")
        .arg("test")
        .arg("--quiet")
        .arg("--")
        .args(test_args)
        .current_dir(manifest_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn \"cargo test\" process")
}

/// Waits for process termination. Panics if timeout has elapsed.
fn wait_for_process_termination(mut child: Child, timeout: Duration) -> ExitStatus {
    let exit_status = child
        .wait_timeout(timeout)
        .expect("Failed to wait for child termination");

    exit_status.unwrap_or_else(|| {
        panic!(
            "Process running for too long. Allowed execution time is {}s.",
            timeout.as_secs()
        )
    })
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
