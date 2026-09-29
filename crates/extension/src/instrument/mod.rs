//! Classify observed PHP calls and start/end spans.

use std::cell::RefCell;

use otel_auto_core::parse::parse_grpc_path;
use otel_auto_core::span::SpanKind;
use otel_auto_core::sql_verb::{sql_operation, sql_span_name};
use otel_auto_core::truncate::truncate_statement;
use ext_php_rs::types::{ArrayKey, ZendCallable, ZendHashTable, Zval};
use ext_php_rs::zend::{ExecuteData, FcallInfo};

use crate::tracer;

thread_local! {
    static REDIS_FUNC: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    GrpcSimpleRequest,
    GrpcInvoker,
    PdoQuery,
    PdoExec,
    PdoStatementExecute,
    RedisCmd,
    CurlExec,
}

pub fn classify(info: &FcallInfo<'_>) -> Option<HookKind> {
    let func = info.function_name?;
    let class = info.class_name.unwrap_or("");

    if class == "Grpc\\BaseStub" && func == "_simpleRequest" {
        return Some(HookKind::GrpcSimpleRequest);
    }
    if is_grpc_invoker(class) && (func == "invoke" || func == "Invoke") {
        return Some(HookKind::GrpcInvoker);
    }
    if class == "PDO" && func == "query" {
        return Some(HookKind::PdoQuery);
    }
    if class == "PDO" && func == "exec" {
        return Some(HookKind::PdoExec);
    }
    if (class == "PDOStatement" || class.ends_with("\\PDOStatement")) && func == "execute" {
        return Some(HookKind::PdoStatementExecute);
    }
    if (class == "Redis" || class == "RedisCluster") && is_redis_cmd(func) {
        return Some(HookKind::RedisCmd);
    }
    if class.is_empty() && func == "curl_exec" {
        return Some(HookKind::CurlExec);
    }
    None
}

/// Match Spiral RR Invoker and alanbase `ServiceCore\Grpc\Invoker` (case-insensitive Grpc/GRPC).
fn is_grpc_invoker(class: &str) -> bool {
    if class == "Spiral\\RoadRunner\\GRPC\\Invoker" || class == "ServiceCore\\Grpc\\Invoker" {
        return true;
    }
    let lower = class.to_ascii_lowercase();
    if lower.ends_with("\\grpc\\invoker") {
        return true;
    }
    lower.contains("roadrunner") && lower.contains("grpc") && lower.ends_with("invoker")
}

fn is_redis_cmd(func: &str) -> bool {
    matches!(
        func.to_ascii_lowercase().as_str(),
        "get" | "set" | "setex" | "setnx"
            | "del" | "delete" | "exists" | "expire"
            | "hget" | "hset" | "hmget" | "hmset" | "hgetall"
            | "lpush" | "rpush" | "lpop" | "rpop" | "lrange"
            | "sadd" | "srem" | "smembers" | "sismember"
            | "zadd" | "zrange" | "zrem"
            | "incr" | "decr" | "incrby" | "decrby"
            | "mget" | "mset" | "keys" | "scan"
            | "publish" | "subscribe"
            | "eval" | "evalsha"
            | "rawcommand"
    )
}

pub fn begin_hook(kind: HookKind, execute_data: &ExecuteData, func_name: Option<&str>) {
    if kind == HookKind::RedisCmd {
        REDIS_FUNC.with(|c| {
            *c.borrow_mut() = func_name.map(|s| s.to_string());
        });
    }
    match kind {
        HookKind::GrpcSimpleRequest => begin_grpc_client(execute_data),
        HookKind::GrpcInvoker => begin_grpc_server(execute_data),
        HookKind::PdoQuery | HookKind::PdoExec => begin_pdo(execute_data),
        HookKind::PdoStatementExecute => begin_pdo_stmt(execute_data),
        HookKind::RedisCmd => begin_redis(execute_data),
        HookKind::CurlExec => begin_curl(),
    }
}

pub fn end_hook(kind: HookKind, _execute_data: &ExecuteData, retval: Option<&Zval>) {
    let err = match kind {
        HookKind::PdoQuery | HookKind::PdoExec | HookKind::PdoStatementExecute => {
            retval.and_then(|z| {
                if z.is_false() {
                    Some("pdo returned false".to_string())
                } else {
                    None
                }
            })
        }
        HookKind::CurlExec => retval.and_then(|z| {
            if z.is_false() {
                Some("curl_exec failed".to_string())
            } else {
                None
            }
        }),
        _ => None,
    };
    if let Some(msg) = err {
        tracer::end_current_error(msg);
    } else {
        tracer::end_current_ok();
    }
    // RR Invoker is a per-RPC job boundary: flush OTLP when the job ends.
    if kind == HookKind::GrpcInvoker {
        tracer::on_request_end();
    }
    REDIS_FUNC.with(|c| *c.borrow_mut() = None);
}

fn arg_zval(execute_data: &ExecuteData, n: usize) -> Option<&mut Zval> {
    unsafe { execute_data.zend_call_arg(n) }
}

fn arg_string(execute_data: &ExecuteData, n: usize) -> Option<String> {
    let z = arg_zval(execute_data, n)?;
    if let Some(s) = z.string() {
        return Some(s);
    }
    z.long().map(|i| i.to_string())
}

fn begin_grpc_client(execute_data: &ExecuteData) {
    let path = arg_string(execute_data, 0).unwrap_or_default();
    let parsed = parse_grpc_path(&path);
    let name = parsed
        .as_ref()
        .map(|p| p.span_name())
        .unwrap_or_else(|| {
            if path.is_empty() {
                "grpc.call".into()
            } else {
                path.clone()
            }
        });
    if !tracer::start_span(name, SpanKind::Client) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("rpc.system", "grpc");
        if let Some(p) = &parsed {
            s.set_attr("rpc.service", p.service.clone());
            s.set_attr("rpc.method", p.method.clone());
        } else if !path.is_empty() {
            s.set_attr("rpc.method", path);
        }
    });
    // Inject W3C Traceparent into gRPC metadata (arg 3) so RR servers can continue.
    inject_traceparent_metadata(execute_data);
}

fn inject_traceparent_metadata(execute_data: &ExecuteData) {
    let Some(tp) = tracer::current_traceparent() else {
        return;
    };
    let Some(meta) = arg_zval(execute_data, 3) else {
        return;
    };
    if meta.array().is_none() {
        meta.set_hashtable(ZendHashTable::new());
    }
    let Some(ht) = meta.array_mut() else {
        return;
    };
    // Do not overwrite caller-supplied traceparent.
    if ht.get("traceparent").is_some() {
        return;
    }
    let mut values = ZendHashTable::new();
    if values.insert_at_index(0, tp.as_str()).is_err() {
        return;
    }
    let _ = ht.insert("traceparent", values);
}

fn begin_grpc_server(execute_data: &ExecuteData) {
    // invoke(ServiceInterface $service, Method $method, ContextInterface $ctx, ?string $input)
    let tp = context_traceparent(execute_data);
    tracer::on_request_start(tp.as_deref());

    let method = method_get_name(execute_data).unwrap_or_else(|| "unknown".into());
    let service = service_rpc_name(execute_data).unwrap_or_else(|| "grpc.service".into());
    let name = format!("{service}/{method}");

    if !tracer::start_span(name, SpanKind::Server) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("rpc.system", "grpc");
        s.set_attr("rpc.service", service);
        s.set_attr("rpc.method", method);
    });
}

fn method_get_name(execute_data: &ExecuteData) -> Option<String> {
    let method_zv = arg_zval(execute_data, 1)?;
    let ret = method_zv.try_call_method("getName", vec![]).ok()?;
    ret.string().filter(|s| !s.is_empty())
}

fn service_rpc_name(execute_data: &ExecuteData) -> Option<String> {
    let service_zv = arg_zval(execute_data, 0)?;
    // Prefer interface NAME constant (e.g. CommonServiceProto.CommonService).
    if let Some(name) = interface_name_constant(service_zv) {
        return Some(name);
    }
    service_zv
        .object()
        .and_then(|o| o.get_class_name().ok())
        .filter(|s| !s.is_empty())
}

fn interface_name_constant(service_zv: &Zval) -> Option<String> {
    let class_implements = ZendCallable::try_from_name("class_implements").ok()?;
    let ifaces = class_implements.try_call(vec![service_zv]).ok()?;
    let ht = ifaces.array()?;
    let constant_fn = ZendCallable::try_from_name("constant").ok()?;
    for (_k, iface_zv) in ht.iter() {
        let iface = iface_zv
            .str()
            .map(str::to_owned)
            .or_else(|| iface_zv.string())?;
        if iface.is_empty() {
            continue;
        }
        let const_path = format!("{iface}::NAME");
        let Ok(val) = constant_fn.try_call(vec![&const_path]) else {
            continue;
        };
        if let Some(s) = val.string().filter(|s| !s.is_empty()) {
            return Some(s);
        }
        if let Some(s) = val.str().filter(|s| !s.is_empty()) {
            return Some(s.to_owned());
        }
    }
    None
}

fn context_traceparent(execute_data: &ExecuteData) -> Option<String> {
    let ctx = arg_zval(execute_data, 2)?;
    // Prefer getValue('traceparent'); RR metadata values are array<string>.
    if let Ok(val) = ctx.try_call_method("getValue", vec![&"traceparent"]) {
        if let Some(s) = first_metadata_string(&val) {
            return Some(s);
        }
    }
    // Fallback: scan getValues() for a traceparent-like key.
    let Ok(values) = ctx.try_call_method("getValues", vec![]) else {
        return None;
    };
    let ht = values.array()?;
    for (key, el) in ht.iter() {
        let key_s = match key {
            ArrayKey::String(s) => s.to_ascii_lowercase(),
            _ => continue,
        };
        if key_s == "traceparent" || key_s.ends_with("traceparent") {
            if let Some(s) = first_metadata_string(el) {
                return Some(s);
            }
        }
    }
    None
}

fn first_metadata_string(z: &Zval) -> Option<String> {
    if let Some(s) = z.string().filter(|s| !s.is_empty()) {
        return Some(s);
    }
    if let Some(s) = z.str().filter(|s| !s.is_empty()) {
        return Some(s.to_owned());
    }
    let ht = z.array()?;
    for (_k, el) in ht.iter() {
        if let Some(s) = el.string().filter(|s| !s.is_empty()) {
            return Some(s);
        }
        if let Some(s) = el.str().filter(|s| !s.is_empty()) {
            return Some(s.to_owned());
        }
    }
    None
}

fn begin_pdo(execute_data: &ExecuteData) {
    let sql = arg_string(execute_data, 0).unwrap_or_default();
    start_pdo_span(&sql);
}

fn begin_pdo_stmt(execute_data: &ExecuteData) {
    let sql = pdo_statement_query_string(execute_data).unwrap_or_default();
    start_pdo_span(&sql);
}

fn start_pdo_span(sql: &str) {
    let name = sql_span_name(sql);
    let op = sql_operation(sql);
    if !tracer::start_span(name, SpanKind::Client) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("db.system", "postgresql");
        s.set_attr("db.operation", op);
        if !sql.is_empty() {
            s.set_attr("db.statement", truncate_statement(sql, 512));
        }
    });
}

/// Read public `PDOStatement::$queryString` (prepared SQL with placeholders).
fn pdo_statement_query_string(execute_data: &ExecuteData) -> Option<String> {
    let this = execute_data.This.object()?;
    this.get_property::<String>("queryString")
        .ok()
        .filter(|s| !s.is_empty())
}

fn begin_redis(execute_data: &ExecuteData) {
    let func = REDIS_FUNC.with(|c| c.borrow().clone()).unwrap_or_else(|| "cmd".into());
    let cmd = func.to_ascii_uppercase();
    let name = format!("redis.{cmd}");
    if !tracer::start_span(name, SpanKind::Client) {
        return;
    }
    let key = arg_string(execute_data, 0).unwrap_or_default();
    tracer::current_mut(|s| {
        s.set_attr("db.system", "redis");
        s.set_attr("db.operation", cmd.to_ascii_lowercase());
        if !key.is_empty() {
            s.set_attr(
                "db.statement",
                truncate_statement(&format!("{cmd} {key}"), 512),
            );
        }
    });
}

fn begin_curl() {
    if !tracer::start_span("HTTP", SpanKind::Client) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("http.request.method", "GET");
    });
}
