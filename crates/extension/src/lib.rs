#![cfg_attr(windows, feature(abi_vectorcall))]

mod instrument;
mod tracer;

use std::cell::Cell;
use std::os::raw::c_int;

use ext_php_rs::prelude::*;
use ext_php_rs::types::Zval;
use ext_php_rs::zend::{ExecuteData, FcallInfo, FcallObserver};

use instrument::{begin_hook, classify, end_hook, HookKind};

thread_local! {
    static PENDING: Cell<Option<HookKind>> = const { Cell::new(None) };
}

/// Auto-instrumentation observer.
pub struct AlanbaseObserver;

impl FcallObserver for AlanbaseObserver {
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
    let method = std::env::var("REQUEST_METHOD").unwrap_or_else(|_| "CLI".into());
    let path = std::env::var("REQUEST_URI")
        .or_else(|_| std::env::var("SCRIPT_NAME"))
        .unwrap_or_else(|_| "/".into());
    if method != "CLI" {
        tracer::start_http_root(&method, path.split('?').next().unwrap_or("/"));
    } else {
        let _ = tracer::start_span("php.request", alanbase_otel_core::span::SpanKind::Server);
    }
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
        .name("alanbase_otel")
        .request_startup_function(request_startup)
        .request_shutdown_function(request_shutdown)
        .fcall_observer(|| AlanbaseObserver)
}
