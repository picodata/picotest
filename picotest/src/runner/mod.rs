use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

mod client;
mod server;

static IS_SERVER_SIDE: OnceLock<bool> = OnceLock::new();
static RUNNERS_MAP: LazyLock<Mutex<HashMap<String,Arc<dyn PicotestRunner>>>> = LazyLock::new(|| {
    Mutex::new(HashMap::with_capacity(1))
});

pub fn running_as_server() -> bool {
    *IS_SERVER_SIDE.get_or_init(detect_run_as_server)
}

fn detect_run_as_server() -> bool {
    let exe_path = std::env::current_exe().unwrap();
    exe_path.ends_with("picodata")
}

pub type UnitTestLocator = extern "C" fn();

pub enum TestStatus {
    Success,
    Failure,
}

pub struct TestResult {
    pub status: TestStatus,
}

pub trait PicotestRunner: Sync + Send {
    fn execute_unit(&self, name: &str, locator_name: &str) -> TestResult;
}

pub fn get_test_runner(package_name: &str) -> Arc<dyn PicotestRunner> {
    assert!(!running_as_server());

    let mut runners_map = RUNNERS_MAP.lock().unwrap();
    if let Some(package_test_runner) = runners_map.get(package_name) {
        return Arc::clone(package_test_runner)
    }
    let new_runner = Arc::new(client::create_test_runner(package_name)) as Arc<dyn PicotestRunner>;
    runners_map.insert(String::from(package_name), Arc::clone(&new_runner));
    new_runner
}