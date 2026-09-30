//! Request-scoped tracer state for the PHP extension (NTS-safe via thread locals).

use std::cell::RefCell;
use std::sync::OnceLock;

use otel_auto_core::config::OtelConfig;
use otel_auto_core::export::export_spans;
use otel_auto_core::span::{SpanKind, StartedSpan, TraceContext};

static CONFIG: OnceLock<OtelConfig> = OnceLock::new();

thread_local! {
    static ACTIVE: RefCell<Vec<StartedSpan>> = const { RefCell::new(Vec::new()) };
    static FINISHED: RefCell<Vec<StartedSpan>> = const { RefCell::new(Vec::new()) };
    static SAMPLED: RefCell<bool> = const { RefCell::new(false) };
    /// Remote parent from inbound `traceparent` (consumed by the first root span).
    static REMOTE_PARENT: RefCell<Option<TraceContext>> = const { RefCell::new(None) };
}

pub fn init_config() {
    let _ = CONFIG.set(OtelConfig::from_env());
}

pub fn config() -> &'static OtelConfig {
    CONFIG.get_or_init(OtelConfig::from_env)
}

pub fn on_request_start(traceparent: Option<&str>) {
    ACTIVE.with(|a| a.borrow_mut().clear());
    FINISHED.with(|f| f.borrow_mut().clear());
    let remote = traceparent.and_then(TraceContext::from_traceparent);
    // Always continue a valid inbound trace so go_test ↔ PHP share one TraceID.
    let sample = if remote.is_some() {
        true
    } else {
        config().should_sample()
    };
    REMOTE_PARENT.with(|r| *r.borrow_mut() = remote);
    SAMPLED.with(|s| *s.borrow_mut() = sample);
}

pub fn is_sampled() -> bool {
    SAMPLED.with(|s| *s.borrow())
}

pub fn on_request_end() {
    // End any leftover active spans.
    ACTIVE.with(|a| {
        let mut stack = a.borrow_mut();
        while let Some(mut span) = stack.pop() {
            if span.end_unix_nano.is_none() {
                span.end_ok();
            }
            FINISHED.with(|f| f.borrow_mut().push(span));
        }
    });
    flush();
}

fn flush() {
    if !is_sampled() {
        FINISHED.with(|f| f.borrow_mut().clear());
        return;
    }
    let spans = FINISHED.with(|f| std::mem::take(&mut *f.borrow_mut()));
    if spans.is_empty() {
        return;
    }
    if let Err(e) = export_spans(config(), &spans) {
        eprintln!("otel_auto: export failed: {e}");
    }
}

/// Start a child span (or root if stack empty). Returns false if not sampling.
pub fn start_span(name: impl Into<String>, kind: SpanKind) -> bool {
    if !is_sampled() {
        return false;
    }
    ACTIVE.with(|a| {
        let mut stack = a.borrow_mut();
        let ctx = match stack.last() {
            Some(parent) => TraceContext::child_of(&parent.ctx),
            None => {
                let remote = REMOTE_PARENT.with(|r| r.borrow_mut().take());
                match remote {
                    Some(parent) => TraceContext::child_of(&parent),
                    None => TraceContext::new_root(),
                }
            }
        };
        stack.push(StartedSpan::start(name, kind, ctx));
        true
    })
}

pub fn current_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut StartedSpan) -> R,
{
    ACTIVE.with(|a| {
        let mut stack = a.borrow_mut();
        stack.last_mut().map(f)
    })
}

pub fn end_current_ok() {
    ACTIVE.with(|a| {
        let mut stack = a.borrow_mut();
        if let Some(mut span) = stack.pop() {
            span.end_ok();
            FINISHED.with(|f| f.borrow_mut().push(span));
        }
    });
}

pub fn end_current_error(msg: impl Into<String>) {
    ACTIVE.with(|a| {
        let mut stack = a.borrow_mut();
        if let Some(mut span) = stack.pop() {
            span.end_error(msg);
            FINISHED.with(|f| f.borrow_mut().push(span));
        }
    });
}

pub fn current_traceparent() -> Option<String> {
    ACTIVE.with(|a| a.borrow().last().map(|s| s.ctx.traceparent()))
}

/// Start SERVER root span with the given display name and optional HTTP attrs.
pub fn start_root(name: impl Into<String>, method: Option<&str>, path: Option<&str>) {
    // Empty ACTIVE stack: parent_span_id Some ⇒ continued from inbound traceparent.
    let will_continue = REMOTE_PARENT.with(|r| r.borrow().is_some());
    if !start_span(name, SpanKind::Server) {
        return;
    }
    current_mut(|s| {
        if will_continue {
            s.set_attr("otel.traceparent.continued", "true");
        }
        if let Some(m) = method {
            s.set_attr("http.request.method", m);
        }
        if let Some(p) = path {
            s.set_attr("url.path", p);
        }
    });
}
