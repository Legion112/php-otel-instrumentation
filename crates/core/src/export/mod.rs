//! OTLP/HTTP JSON exporter (blocking) for Alloy/Tempo.

use serde_json::{json, Value};

use crate::config::OtelConfig;
use crate::span::StartedSpan;

/// Export completed spans to OTLP HTTP JSON endpoint.
pub fn export_spans(cfg: &OtelConfig, spans: &[StartedSpan]) -> Result<(), String> {
    if spans.is_empty() || !cfg.enabled {
        return Ok(());
    }
    let body = build_otlp_json(&cfg.service_name, spans);
    let url = cfg.traces_url();
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| format!("otlp post {url}: {e}"))?;
    let status = resp.status();
    if !(200..300).contains(&status) {
        let text = resp.into_string().unwrap_or_default();
        return Err(format!("otlp status {status}: {text}"));
    }
    Ok(())
}

/// Build OTLP JSON ExportTraceServiceRequest body.
pub fn build_otlp_json(service_name: &str, spans: &[StartedSpan]) -> Value {
    let otlp_spans: Vec<Value> = spans
        .iter()
        .map(|s| {
            let end = s.end_unix_nano.unwrap_or(s.start_unix_nano);
            let mut attrs: Vec<Value> = s
                .attributes
                .iter()
                .map(|(k, v)| {
                    json!({
                        "key": k,
                        "value": { "stringValue": v }
                    })
                })
                .collect();
            // Ensure rpc/db semantic attrs stay as strings; already in map.
            let _ = &mut attrs;

            let mut span = json!({
                "traceId": s.ctx.trace_id_hex(),
                "spanId": s.ctx.span_id_hex(),
                "name": s.name,
                "kind": s.kind.as_otlp(),
                "startTimeUnixNano": s.start_unix_nano.to_string(),
                "endTimeUnixNano": end.to_string(),
                "attributes": attrs,
                "status": {
                    "code": s.status.as_otlp_code()
                }
            });
            if let Some(parent) = s.ctx.parent_span_id {
                span["parentSpanId"] = json!(hex::encode(parent));
            }
            if let crate::span::SpanStatus::Error(msg) = &s.status {
                span["status"]["message"] = json!(msg);
            }
            span
        })
        .collect();

    json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [{
                    "key": "service.name",
                    "value": { "stringValue": service_name }
                }]
            },
            "scopeSpans": [{
                "scope": {
                    "name": "alanbase-otel",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "spans": otlp_spans
            }]
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{SpanKind, TraceContext};

    #[test]
    fn json_contains_service_and_span_name() {
        let mut span = crate::span::StartedSpan::start(
            "LinkServiceProto.LinkService/getTargetLink",
            SpanKind::Client,
            TraceContext::new_root(),
        );
        span.set_attr("rpc.system", "grpc");
        span.end_ok();
        let body = build_otlp_json("alanbase", &[span]);
        let s = body.to_string();
        assert!(s.contains("alanbase"));
        assert!(s.contains("LinkServiceProto.LinkService/getTargetLink"));
        assert!(s.contains("rpc.system"));
    }
}
