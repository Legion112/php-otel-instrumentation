//! Pure Rust core: config, path parsing, span model, OTLP/HTTP export.

pub mod config;
pub mod export;
pub mod parse;
pub mod request_name;
pub mod span;
pub mod truncate;

pub use config::OtelConfig;
pub use parse::grpc_path::{parse_grpc_path, GrpcPath};
pub use request_name::root_span_name;
pub use span::{SpanKind, SpanStatus, StartedSpan, TraceContext};
pub use truncate::truncate_statement;
