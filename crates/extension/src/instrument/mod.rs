//! Classify observed PHP calls and start/end spans.

use std::cell::RefCell;

use alanbase_otel_core::parse::parse_grpc_path;
use alanbase_otel_core::span::SpanKind;
use alanbase_otel_core::truncate::truncate_statement;
use ext_php_rs::types::Zval;
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
    // Spiral RoadRunner GRPC Invoker (inbound worker dispatch).
    if (class == "Spiral\\RoadRunner\\GRPC\\Invoker"
        || class.ends_with("\\GRPC\\Invoker")
        || (class.contains("RoadRunner") && class.contains("GRPC") && class.ends_with("Invoker")))
        && (func == "invoke" || func == "Invoke")
    {
        return Some(HookKind::GrpcInvoker);
    }
    if class == "PDO" && func == "query" {
        return Some(HookKind::PdoQuery);
    }
    if class == "PDO" && func == "exec" {
        return Some(HookKind::PdoExec);
    }
    if class == "PDOStatement" && func == "execute" {
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
        HookKind::PdoQuery | HookKind::PdoExec => begin_pdo(execute_data, kind),
        HookKind::PdoStatementExecute => begin_pdo_stmt(),
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
    REDIS_FUNC.with(|c| *c.borrow_mut() = None);
}

fn arg_string(execute_data: &ExecuteData, n: usize) -> Option<String> {
    unsafe {
        let z = execute_data.zend_call_arg(n)?;
        if let Some(s) = z.string() {
            return Some(s);
        }
        z.long().map(|i| i.to_string())
    }
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
}

fn begin_grpc_server(execute_data: &ExecuteData) {
    let service = arg_string(execute_data, 0).unwrap_or_else(|| "grpc.service".into());
    let method = arg_string(execute_data, 1).unwrap_or_else(|| "unknown".into());
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

fn begin_pdo(execute_data: &ExecuteData, kind: HookKind) {
    let sql = arg_string(execute_data, 0).unwrap_or_default();
    let op = match kind {
        HookKind::PdoExec => "exec",
        _ => "query",
    };
    if !tracer::start_span(format!("db.{op}"), SpanKind::Client) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("db.system", "postgresql");
        s.set_attr("db.operation", op);
        if !sql.is_empty() {
            s.set_attr("db.statement", truncate_statement(&sql, 512));
        }
    });
}

fn begin_pdo_stmt() {
    if !tracer::start_span("db.execute", SpanKind::Client) {
        return;
    }
    tracer::current_mut(|s| {
        s.set_attr("db.system", "postgresql");
        s.set_attr("db.operation", "execute");
    });
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
