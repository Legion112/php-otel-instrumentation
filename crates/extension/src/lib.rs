#![cfg_attr(windows, feature(abi_vectorcall))]

mod instrument;
mod tracer;

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use otel_auto_core::{cli_span_name, prefer_http_path, root_span_name_from_candidates};
use ext_php_rs::prelude::*;
use ext_php_rs::types::Zval;
use ext_php_rs::zend::{ExecuteData, FcallInfo, FcallObserver, ProcessGlobals, SapiGlobals};

// FPM/CGI request env (not process environ). SG(request_info).request_uri is SCRIPT_NAME.
unsafe extern "C" {
    fn sapi_getenv(name: *const c_char, name_len: usize) -> *mut c_char;
}

use instrument::{begin_hook, classify, end_hook, HookKind};

thread_local! {
    /// Stack of in-flight observed calls (must nest; a single Cell drops Invoker ends).
    static PENDING: RefCell<Vec<HookKind>> = const { RefCell::new(Vec::new()) };
}

/// Auto-instrumentation observer.
pub struct OtelAutoObserver;

impl FcallObserver for OtelAutoObserver {
    fn should_observe(&self, info: &FcallInfo) -> bool {
        classify(info).is_some()
    }

    fn begin(&self, execute_data: &ExecuteData) {
        let info = derive_fcall_info(execute_data);
        let Some(kind) = classify(&info) else {
            return;
        };
        PENDING.with(|p| p.borrow_mut().push(kind));
        begin_hook(kind, execute_data, info.function_name);
    }

    fn end(&self, execute_data: &ExecuteData, retval: Option<&Zval>) {
        if let Some(kind) = PENDING.with(|p| p.borrow_mut().pop()) {
            end_hook(kind, execute_data, retval);
        }
    }
}

fn derive_fcall_info(execute_data: &ExecuteData) -> FcallInfo<'_> {
    unsafe {
        let func = (*execute_data).func;
        if func.is_null() {
            return empty_info();
        }
        let common = &(*func).common;
        let function_name = zstr_to_str(common.function_name);
        let class_name = if common.scope.is_null() {
            None
        } else {
            zstr_to_str((*common.scope).name)
        };
        let is_internal = false;
        FcallInfo {
            function_name,
            class_name,
            filename: None,
            lineno: 0,
            is_internal,
        }
    }
}

fn empty_info<'a>() -> FcallInfo<'a> {
    FcallInfo {
        function_name: None,
        class_name: None,
        filename: None,
        lineno: 0,
        is_internal: false,
    }
}

unsafe fn zstr_to_str<'a>(s: *mut ext_php_rs::ffi::zend_string) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    let len = unsafe { (*s).len };
    let ptr = unsafe { (*s).val.as_ptr() };
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len as usize) };
    std::str::from_utf8(bytes).ok()
}

extern "C" fn request_startup(_type: c_int, _module_number: c_int) -> c_int {
    // W3C Trace Context from inbound HTTP (nginx FastCGI → HTTP_TRACEPARENT).
    let traceparent = sapi_env("HTTP_TRACEPARENT")
        .or_else(|| server_header("HTTP_TRACEPARENT"))
        .or_else(|| std::env::var("HTTP_TRACEPARENT").ok());
    tracer::on_request_start(traceparent.as_deref());

    // FPM sets SG(request_info).request_uri to SCRIPT_NAME (/index.php), not the
    // client URI. Real path is FastCGI REQUEST_URI → sapi_getenv / $_SERVER.
    let (sapi_method, sapi_script_name, sapi_script) = {
        let sg = SapiGlobals::get();
        let info = sg.request_info();
        (
            info.request_method().map(str::to_owned),
            info.request_uri().map(str::to_owned), // SCRIPT_NAME under FPM
            info.path_translated().map(str::to_owned),
        )
    };
    let fcgi_uri = sapi_env("REQUEST_URI");
    let fcgi_method = sapi_env("REQUEST_METHOD");
    let (server_uri, server_redirect, server_path_info, server_method) = server_request_fields();

    let method = sapi_method
        .or(fcgi_method)
        .or(server_method)
        .or_else(|| std::env::var("REQUEST_METHOD").ok());
    let process_env_uri = std::env::var("REQUEST_URI").ok();
    let script = sapi_script
        .or_else(|| sapi_env("SCRIPT_FILENAME"))
        .or_else(|| std::env::var("SCRIPT_FILENAME").ok());

    let argv = cli_argv();
    // RoadRunner workers are long-lived CLI processes; do not open a process-lifetime
    // root span. Per-RPC SERVER roots start in Invoker::invoke instead.
    if is_roadrunner_worker(script.as_deref(), &argv) {
        return 0;
    }

    let candidates = [
        fcgi_uri.as_deref(),
        server_uri.as_deref(),
        server_redirect.as_deref(),
        server_path_info.as_deref(),
        process_env_uri.as_deref(),
        // Last resort: SCRIPT_NAME (/index.php) — better than nothing for HTTP.
        sapi_script_name.as_deref(),
    ];
    let method_trimmed = method
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let name = if method_trimmed.is_some() {
        root_span_name_from_candidates(method_trimmed, &candidates, script.as_deref())
    } else {
        cli_span_name(script.as_deref(), &argv)
    };
    let path_attr = method_trimmed.map(|_| prefer_http_path(&candidates).to_string());
    tracer::start_root(
        &name,
        method_trimmed,
        path_attr.as_deref(),
    );

    0
}

fn is_roadrunner_worker(script: Option<&str>, argv: &[String]) -> bool {
    let looks_like_worker = |s: &str| {
        let lower = s.to_ascii_lowercase();
        lower.ends_with("worker.php") || lower.contains("/worker.php") || lower.contains("\\worker.php")
    };
    if script.map(looks_like_worker).unwrap_or(false) {
        return true;
    }
    argv.iter().any(|a| looks_like_worker(a))
}

/// CLI argv for span naming.
///
/// Prefer `SG(request_info).argv` (PHP CLI script + args). Fall back to process
/// args / `$_SERVER['argv']`, picking the longest when SAPI only has the script.
fn cli_argv() -> Vec<String> {
    let sapi = sapi_cli_argv();
    if sapi.len() > 1 {
        return sapi;
    }
    let env: Vec<String> = std::env::args().collect();
    let server = server_cli_argv();
    [sapi, env, server]
        .into_iter()
        .max_by_key(|v| v.len())
        .unwrap_or_default()
}

/// `SG(request_info).argv[0..argc]` (PHP CLI script + args).
fn sapi_cli_argv() -> Vec<String> {
    let sg = SapiGlobals::get();
    let info = sg.request_info();
    let argc = info.argc;
    if argc <= 0 || info.argv.is_null() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(argc as usize);
    unsafe {
        let argv = std::slice::from_raw_parts(info.argv, argc as usize);
        for &ptr in argv {
            if ptr.is_null() {
                continue;
            }
            if let Ok(s) = CStr::from_ptr(ptr).to_str() {
                if !s.is_empty() {
                    out.push(s.to_owned());
                }
            }
        }
    }
    out
}

fn server_cli_argv() -> Vec<String> {
    let pg = ProcessGlobals::get();
    let Some(server) = pg.http_server_vars() else {
        return Vec::new();
    };
    let Some(zv) = server.get("argv") else {
        return Vec::new();
    };
    let Some(ht) = zv.array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (_key, el) in ht.iter() {
        if let Some(s) = el.str() {
            out.push(s.to_owned());
        } else if let Some(s) = el.string() {
            out.push(s);
        }
        if out.len() > 256 {
            break;
        }
    }
    out
}

fn sapi_env(key: &str) -> Option<String> {
    let c_name = CString::new(key).ok()?;
    unsafe {
        let ptr = sapi_getenv(c_name.as_ptr(), key.len());
        if ptr.is_null() {
            return None;
        }
        // FPM returns a pointer into the request env; do not free.
        CStr::from_ptr(ptr).to_str().ok().map(str::to_owned)
    }
}

fn server_request_fields() -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let pg = ProcessGlobals::get();
    let Some(server) = pg.http_server_vars() else {
        return (None, None, None, None);
    };
    let get = |key: &str| server.get(key).and_then(|z| z.string());
    (
        get("REQUEST_URI"),
        get("REDIRECT_URL"),
        get("PATH_INFO"),
        get("REQUEST_METHOD"),
    )
}

fn server_header(key: &str) -> Option<String> {
    let pg = ProcessGlobals::get();
    let server = pg.http_server_vars()?;
    server.get(key).and_then(|z| z.string())
}

extern "C" fn request_shutdown(_type: c_int, _module_number: c_int) -> c_int {
    tracer::on_request_end();
    0
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    tracer::init_config();
    module
        .name("otel_auto")
        .request_startup_function(request_startup)
        .request_shutdown_function(request_shutdown)
        .fcall_observer(|| OtelAutoObserver)
}
