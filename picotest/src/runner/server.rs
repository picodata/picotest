use std::backtrace::{self, Backtrace, BacktraceStatus};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, Location, PanicHookInfo, UnwindSafe};
use std::sync::OnceLock;

use base64::prelude::BASE64_STANDARD_NO_PAD;
use base64::Engine;
use libloading::os::unix::{Library, Symbol};

use crate::internal::{plugin_dylib_path, plugin_root_dir};

unsafe extern "C" {
    fn fiber_id(fiber: *mut c_char) -> u64;
    fn fiber_set_name_n(fiber: *mut c_char, name: *const c_char, len: u32);
}

type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + 'static + Sync + Send>;

struct PicotestPanicLocation {
    file: String,
    line: u32,
    col: u32,
}

impl From<&Location<'_>> for PicotestPanicLocation {
    fn from(value: &Location) -> Self {
        Self {
            file: value.file().to_string(),
            line: value.line(),
            col: value.column(),
        }
    }
}

pub struct PicotestPanicInfo {
    payload_str: String,
    backtrace: Backtrace,
    location: Option<PicotestPanicLocation>,
}

impl PicotestPanicInfo {
    fn encode_with_base64(&self) -> String {
        let mut output = String::with_capacity(512);
        output += "payload:";
        BASE64_STANDARD_NO_PAD.encode_string(&self.payload_str, &mut output);
        if let Some(location) = &self.location {
            output += ";location:";
            let loc_short = format!("{}:{}:{}", location.file, location.line, location.col);
            BASE64_STANDARD_NO_PAD.encode_string(loc_short, &mut output);
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            output += ";backtrace:";
            let backtrace_str = match std::env::var("RUST_BACKTRACE") {
                Ok(s) if s == "full" => format!("{:#}",self.backtrace),
                _ => format!("{}",self.backtrace),
            };
            BASE64_STANDARD_NO_PAD.encode_string(backtrace_str.strip_suffix("\n").unwrap(), &mut output);
        }
        output += ";";
        output
    }

    pub fn decode_with_base64(data: &str) -> (String,Option<String>,Option<String>) {
        assert!(data.starts_with("payload:"));
        let tail = data.strip_prefix("payload:").unwrap();
        let (payload_value,mut tail) = tail.split_once(";").unwrap();
        let payload = String::from_utf8(BASE64_STANDARD_NO_PAD.decode(payload_value).unwrap()).unwrap();
        
        let location = if tail.starts_with("location:") {
            tail = tail.strip_prefix("location:").unwrap();
            let (location_value, new_tail) = tail.split_once(';').unwrap();
            let location = String::from_utf8(BASE64_STANDARD_NO_PAD.decode(location_value).unwrap()).unwrap();
            tail = new_tail;
            Some(location)
        } else {
            None
        };

        let backtrace = if tail.starts_with("backtrace:") {
            tail = tail.strip_prefix("backtrace:").unwrap();
            let (backtrace_value, _) = tail.split_once(';').unwrap();
            Some(String::from_utf8(BASE64_STANDARD_NO_PAD.decode(backtrace_value).unwrap()).unwrap())
        } else {
            None
        };

        (payload,location,backtrace)
    }
}

thread_local! {
    static RAISED_PANICS: RefCell<HashMap<u64,PicotestPanicInfo>> = RefCell::new(HashMap::with_capacity(16));
    static GUARDED_FIBERS: RefCell<HashSet<u64>> = RefCell::new(HashSet::with_capacity(100))
}

static PICOPLUGIN_HANDLER: OnceLock<PanicHook> = OnceLock::new();

fn install_picotest_panic_hook() {
    let mut first_install = false;
    PICOPLUGIN_HANDLER.get_or_init(|| {
        first_install = true;
        std::panic::take_hook()
    });
    if first_install {
        let boxed_hook = Box::new(picotest_panic_hook);
        std::panic::set_hook(boxed_hook);
    }
}

fn picotest_panic_hook(info: &PanicHookInfo<'_>) {
    let current_id = unsafe { fiber_id(std::ptr::null_mut()) };
    let is_guarded = GUARDED_FIBERS.with(|set_cell| set_cell.borrow().contains(&current_id));
    if !is_guarded {
        let original_handler = PICOPLUGIN_HANDLER
            .get()
            .expect("install_hook must extract original handler");
        original_handler.as_ref()(info);
    }

    let backtrace = backtrace::Backtrace::capture();
    let location = info.location().map(|l| PicotestPanicLocation::from(l));
    let payload_str = if let Some(s) = info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.to_string()
    } else {
        String::from("unknown panic")
    };
    RAISED_PANICS.with_borrow_mut(move |state| {
        state.insert(
            current_id,
            PicotestPanicInfo {
                backtrace,
                payload_str,
                location,
            },
        );
    });
    // trigger unwinding to std::panic::catch_unwind by exiting this handler
}

fn fiber_catch_unwind<F, R>(f: F) -> Result<R, PicotestPanicInfo>
where
    F: FnOnce() -> R + UnwindSafe,
{
    install_picotest_panic_hook();
    let current_fiber = unsafe { fiber_id(std::ptr::null_mut()) };
    GUARDED_FIBERS.with(|map_cell| map_cell.borrow_mut().insert(current_fiber));
    let result = catch_unwind(f);
    GUARDED_FIBERS.with(|map_cell| map_cell.borrow_mut().remove(&current_fiber));
    result.map_err(|_| RAISED_PANICS.with_borrow_mut(|map| map.remove(&current_fiber).unwrap()))
}

#[repr(C)]
pub struct PicounitResult {
    fail: u8,
    data: *mut c_char,
    len: u32,
    cap: u32,
}

impl Default for PicounitResult {
    fn default() -> Self {
        Self {
            fail: 0,
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }
}

impl PicounitResult {
    fn failure(err: PicotestPanicInfo) -> Self {
        let mut data_string = err.encode_with_base64();
        let len = data_string.len();
        let cap = data_string.capacity();
        let data = data_string.as_mut_ptr();
        std::mem::forget(data_string);
        Self {
            fail: 1,
            data: data as *mut i8,
            len: len as u32,
            cap: cap as u32,
        }
    }
}

#[allow(unused)]
#[unsafe(no_mangle)]
unsafe extern "C" fn picotest_free_unit_result(result: PicounitResult) {
    if result.data != std::ptr::null_mut() {
        let data_string =
            String::from_raw_parts(result.data as _, result.len as _, result.cap as _);
        drop(data_string)
    }
}

#[allow(unused)]
#[unsafe(no_mangle)]
unsafe extern "C" fn picotest_execute_unit(
    package: *const c_char,
    name: *const c_char,
    locator_name: *const c_char,
) -> PicounitResult {
    let package = CStr::from_ptr(package).to_str().unwrap().to_string();
    let name_str = CStr::from_ptr(name).to_str().unwrap().to_string();
    let locator_name = CStr::from_ptr(locator_name).to_str().unwrap().to_string();

    let plugin_path = plugin_root_dir();
    let dylib_path = plugin_dylib_path(&plugin_path, &package);
    let dylib_path = dylib_path.to_str().unwrap();

    let test_lib = Library::new(dylib_path).unwrap();
    let locator_fn: Symbol<extern "C" fn() -> fn()> =
        test_lib.get(locator_name.as_bytes()).unwrap();
    let test_fn = (locator_fn)();

    fiber_set_name_n(std::ptr::null_mut(), name, name_str.len() as u32);

    let result = fiber_catch_unwind(|| test_fn());

    match result {
        Ok(..) => PicounitResult::default(),
        Err(error) => PicounitResult::failure(error),
    }
}
