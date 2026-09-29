#![cfg_attr(windows, feature(abi_vectorcall))]

mod instrument;
mod tracer;

use std::cell::Cell;
use std::os::raw::c_int;

use otel_auto_core::root_span_name;
use ext_php_rs::prelude::*;
use ext_php_rs::types::Zval;
use ext_php_rs::zend::{ExecuteData, FcallInfo, FcallObserver, SapiGlobals};

use instrument::{begin_hook, classify, end_hook, HookKind};

thread_local! {
    static PENDING: Cell<Option<HookKind>> = const { Cell::new(None) };
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
        PENDING.with(|p| p.set(Some(kind)));
        begin_hook(kind, execute_data, info.function_name);
    }

    fn end(&self, execute_data: &ExecuteData, retval: Option<&Zval>) {
        if let Some(kind) = PENDING.with(|p| p.take()) {
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
    tracer::on_request_start();

    // Prefer SAPI request_info (FPM/FastCGI); process env is only a test fallback.
    let (sapi_method, sapi_uri, sapi_script) = {
        let sg = SapiGlobals::get();
        let info = sg.request_info();
        (
            info.request_method().map(str::to_owned),
            info.request_uri().map(str::to_owned),
            info.path_translated().map(str::to_owned),
        )
    };
    let method = sapi_method.or_else(|| std::env::var("REQUEST_METHOD").ok());
    let uri = sapi_uri
        .or_else(|| std::env::var("REQUEST_URI").ok())
        .or_else(|| std::env::var("SCRIPT_NAME").ok());
    let script = sapi_script.or_else(|| std::env::var("SCRIPT_FILENAME").ok());

    let name = root_span_name(
        method.as_deref(),
        uri.as_deref(),
        script.as_deref(),
    );
    let path_attr = method.as_ref().map(|_| {
        uri.as_deref()
            .or(script.as_deref())
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/")
    });
    tracer::start_root(&name, method.as_deref(), path_attr);

    0
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
