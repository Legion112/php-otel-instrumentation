//! In-process span model and W3C trace context helpers.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

/// OpenTelemetry span kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Internal,
    Server,
    Client,
}

impl SpanKind {
    pub fn as_otlp(self) -> i32 {
        match self {
            Self::Internal => 1,
            Self::Server => 2,
            Self::Client => 3,
        }
    }
}

/// Span status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error(String),
}

impl SpanStatus {
    pub fn as_otlp_code(&self) -> i32 {
        match self {
            Self::Unset => 0,
            Self::Ok => 1,
            Self::Error(_) => 2,
        }
    }
}

/// Trace/span identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent_span_id: Option<[u8; 8]>,
}

impl TraceContext {
    pub fn new_root() -> Self {
        let mut trace_id = [0u8; 16];
        let mut span_id = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut trace_id);
        rand::thread_rng().fill_bytes(&mut span_id);
        // Ensure non-zero IDs per W3C.
        if trace_id.iter().all(|&b| b == 0) {
            trace_id[15] = 1;
        }
        if span_id.iter().all(|&b| b == 0) {
            span_id[7] = 1;
        }
        Self {
            trace_id,
            span_id,
            parent_span_id: None,
        }
    }

    pub fn child_of(parent: &TraceContext) -> Self {
        let mut span_id = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut span_id);
        if span_id.iter().all(|&b| b == 0) {
            span_id[7] = 1;
        }
        Self {
            trace_id: parent.trace_id,
            span_id,
            parent_span_id: Some(parent.span_id),
        }
    }

    /// Parse a W3C `traceparent` header into a remote parent context.
    ///
    /// Format: `{version}-{trace-id}-{parent-id}-{trace-flags}` (e.g. `00-…-…-01`).
    /// Returns `None` for invalid / all-zero IDs. The returned context uses the
    /// remote parent-id as `span_id` so callers can `child_of` to continue the trace.
    pub fn from_traceparent(header: &str) -> Option<Self> {
        let header = header.trim();
        let parts: Vec<&str> = header.split('-').collect();
        if parts.len() != 4 {
            return None;
        }
        let version = parts[0];
        if version.len() != 2 || version.eq_ignore_ascii_case("ff") {
            return None;
        }
        if parts[1].len() != 32 || parts[2].len() != 16 || parts[3].len() != 2 {
            return None;
        }
        if !parts[1].chars().all(|c| c.is_ascii_hexdigit())
            || !parts[2].chars().all(|c| c.is_ascii_hexdigit())
            || !parts[3].chars().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        let mut trace_id = [0u8; 16];
        let mut span_id = [0u8; 8];
        hex::decode_to_slice(parts[1], &mut trace_id).ok()?;
        hex::decode_to_slice(parts[2], &mut span_id).ok()?;
        if trace_id.iter().all(|&b| b == 0) || span_id.iter().all(|&b| b == 0) {
            return None;
        }
        Some(Self {
            trace_id,
            span_id,
            parent_span_id: None,
        })
    }

    pub fn trace_id_hex(&self) -> String {
        hex::encode(self.trace_id)
    }

    pub fn span_id_hex(&self) -> String {
        hex::encode(self.span_id)
    }

    /// W3C `traceparent` header value.
    pub fn traceparent(&self) -> String {
        format!(
            "00-{}-{}-01",
            self.trace_id_hex(),
            self.span_id_hex()
        )
    }
}

/// A completed or in-flight span record.
#[derive(Debug, Clone)]
pub struct StartedSpan {
    pub name: String,
    pub kind: SpanKind,
    pub ctx: TraceContext,
    pub start_unix_nano: u64,
    pub end_unix_nano: Option<u64>,
    pub attributes: HashMap<String, String>,
    pub status: SpanStatus,
}

impl StartedSpan {
    pub fn start(name: impl Into<String>, kind: SpanKind, ctx: TraceContext) -> Self {
        Self {
            name: name.into(),
            kind,
            ctx,
            start_unix_nano: now_unix_nano(),
            end_unix_nano: None,
            attributes: HashMap::new(),
            status: SpanStatus::Unset,
        }
    }

    pub fn set_attr(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.attributes.insert(key.into(), value.into());
    }

    pub fn end_ok(&mut self) {
        self.end_unix_nano = Some(now_unix_nano());
        if matches!(self.status, SpanStatus::Unset) {
            self.status = SpanStatus::Ok;
        }
    }

    pub fn end_error(&mut self, msg: impl Into<String>) {
        self.end_unix_nano = Some(now_unix_nano());
        self.status = SpanStatus::Error(msg.into());
    }
}

fn now_unix_nano() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_shares_trace_id() {
        let root = TraceContext::new_root();
        let child = TraceContext::child_of(&root);
        assert_eq!(root.trace_id, child.trace_id);
        assert_ne!(root.span_id, child.span_id);
        assert_eq!(child.parent_span_id, Some(root.span_id));
    }

    #[test]
    fn traceparent_format() {
        let ctx = TraceContext::new_root();
        let tp = ctx.traceparent();
        let parts: Vec<_> = tp.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1].len(), 32);
        assert_eq!(parts[2].len(), 16);
        assert_eq!(parts[3], "01");
    }

    #[test]
    fn from_traceparent_roundtrip() {
        let root = TraceContext::new_root();
        let parsed = TraceContext::from_traceparent(&root.traceparent()).unwrap();
        assert_eq!(parsed.trace_id, root.trace_id);
        assert_eq!(parsed.span_id, root.span_id);
        let child = TraceContext::child_of(&parsed);
        assert_eq!(child.trace_id, root.trace_id);
        assert_eq!(child.parent_span_id, Some(root.span_id));
    }

    #[test]
    fn from_traceparent_rejects_invalid() {
        assert!(TraceContext::from_traceparent("").is_none());
        assert!(TraceContext::from_traceparent("00-00-00-01").is_none());
        assert!(TraceContext::from_traceparent(
            "00-00000000000000000000000000000000-b7ad6b7169203331-01"
        )
        .is_none());
        assert!(TraceContext::from_traceparent(
            "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01"
        )
        .is_none());
        assert!(TraceContext::from_traceparent(
            "ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        )
        .is_none());
    }

    #[test]
    fn from_traceparent_valid_example() {
        let tp = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let ctx = TraceContext::from_traceparent(tp).unwrap();
        assert_eq!(ctx.trace_id_hex(), "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(ctx.span_id_hex(), "b7ad6b7169203331");
    }
}
