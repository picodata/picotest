use std::path::PathBuf;

use super::{PicotestRunner, TestResult, TestStatus };
use crate::get_or_create_session_cluster;
use crate::internal::verify_unit_test_output;
use crate::internal::{get_or_create_unit_test_topology, plugin_dylib_path, plugin_root_dir};
use picotest_helpers::Cluster;

struct RemotePicotestRunner {
    #[allow(unused)]
    package_name: String,
    plugin_dylib_path: PathBuf,
    cluster: &'static Cluster,
}

impl PicotestRunner for RemotePicotestRunner {
    fn execute_unit(&self, name: &str, locator_name: &str) -> TestResult {
        let package_name = &self.package_name;
        let dylib_path = self.plugin_dylib_path.to_str().unwrap();
        let call_server_side = format!(
        r#"
"[*] Running unit-test '{name}'"

ffi = require("ffi")
_G.__picotest = _G.__picotest or {{ }}
_G.__picotest_result = _G.__picotest_result or ffi.cdef[[ typedef struct {{ uint8_t fail; char* data; size_t len; size_t cap; }} picounit_result; ]]
_G.__picotest_exec_unit = _G.__picotest_exec_unit or ffi.cdef[[ picounit_result (picotest_execute_unit)(const char*, const char*, const char*); ]]
_G.__picotest_free_unit = _G.__picotest_free_unit or ffi.cdef[[ void (picotest_free_unit_result)(picounit_result); ]]
_G.__picotest["{package_name}"] = _G.__picotest["{package_name}"] or {{ lib = ffi.load("{dylib_path}") }}

result = _G.__picotest["{package_name}"].lib.picotest_execute_unit("{package_name}","{name}","{locator_name}")

"[*] Test '{name}' has been finished"
("picotest_unit|{name}|fail=%s"):format(result.fail)
("picotest_unit|{name}|data=%s"):format(ffi.string(result.data,result.len))

_G.__picotest["{package_name}"].lib.picotest_free_unit_result(result)
true"#
        );

        let output = self.cluster
            .run_lua(call_server_side)
            .expect("Failed to execute query");

        let test_out_prefix = format!("- picotest_unit|{name}|");
        let mut fail = false;
        let (mut payload,mut location,mut backtrace): (String,Option<String>,Option<String>) = (String::new(),None,None);
        for line in output.split("\n") {
            if !line.starts_with(&test_out_prefix) {
                continue
            }
            let line = line.strip_prefix(&test_out_prefix).unwrap();
            if !line.contains("=") {
                continue;
            }
            
            let (key,value) = line.split_once("=").unwrap();
            if key == "fail" && value == "1" {
                fail = true
            }
            if key == "data" {
                (payload, location, backtrace) = super::server::PicotestPanicInfo::decode_with_base64(value);
            }
        }

        if fail {
            let data = {
                let mut out = String::with_capacity(backtrace.as_ref().map(|b| b.len()).unwrap_or(0)+200);
                let location = location.unwrap_or(String::from("<?>"));
                out += &format!("remote fiber panicked at {}:\n{}",location,payload);
                if let Some(backtrace) = backtrace {
                    out += "\nremote stack backtrace:\n";
                    out += &backtrace;
                }
                out
            };
            panic!("{}",data);
        }
        if let Err(err) = verify_unit_test_output(&output) {
            for l in output.split("----") {
                println!("[Lua] {l}")
            }
            panic!("Test '{name}' exited with failure: {}", err);
        }
        TestResult { status: TestStatus::Success }
    }
}

pub fn create_test_runner(package_name: &str) -> impl PicotestRunner {
    let package_name = package_name.to_string();
    let plugin_path = plugin_root_dir();
    let plugin_dylib_path = plugin_dylib_path(&plugin_path, &package_name);
    let plugin_topology = get_or_create_unit_test_topology();

    let cluster = get_or_create_session_cluster(
        plugin_path.to_str().unwrap().into(),
        plugin_topology.into(),
        0,
    );

    RemotePicotestRunner {
        package_name,
        cluster,
        plugin_dylib_path,
    }
}
