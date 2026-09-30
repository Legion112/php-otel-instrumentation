//! Local tooling: PHP integration harness (replaces php-tests/*.sh / *.py).

mod mock_otlp;

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};

use mock_otlp::MockOtlp;

#[derive(Parser)]
#[command(name = "xtask", about = "php-otel local tooling")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run smoke + integration_hooks.php against a mock OTLP receiver.
    PhpIntegration {
        /// Path to otel_auto.so (default: <repo>/otel_auto.so).
        #[arg(long, env = "OTEL_AUTO_SO")]
        so: Option<PathBuf>,
        /// Mock OTLP listen port.
        #[arg(long, env = "OTLP_PORT", default_value_t = 14318)]
        port: u16,
        /// Optional file mirror of received OTLP JSON payloads.
        #[arg(long, env = "OTLP_OUT")]
        out: Option<PathBuf>,
        /// PHP binary (default: php).
        #[arg(long, env = "PHP_BIN", default_value = "php")]
        php: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::PhpIntegration { so, port, out, php } => {
            if let Err(e) = run_php_integration(so, port, out, php) {
                eprintln!("FAIL: {e}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/xtask -> repo root")
        .to_path_buf()
}

fn run_php_integration(
    so: Option<PathBuf>,
    port: u16,
    out: Option<PathBuf>,
    php: PathBuf,
) -> Result<(), String> {
    let root = repo_root();
    let so = so.unwrap_or_else(|| root.join("otel_auto.so"));
    if !so.is_file() {
        return Err(format!(
            "missing {} — build with Dockerfile.build first",
            so.display()
        ));
    }

    let out = out.unwrap_or_else(|| PathBuf::from("/tmp/otel-auto-otlp-spans.json"));
    let mock = MockOtlp::start(port, Some(out.clone())).map_err(|e| e.to_string())?;
    // Brief pause so the listen socket is ready.
    thread::sleep(Duration::from_millis(100));

    let smoke = root.join("php-tests/smoke.php");
    let hooks = root.join("php-tests/integration_hooks.php");
    let cont = root.join("php-tests/continue_trace.php");
    let ext = format!("extension={}", so.display());

    run_php(&php, &ext, &smoke, port, &[])?;
    run_php(&php, &ext, &hooks, port, &[])?;

    // Known inbound Trace Context — SERVER root must continue this TraceID.
    const TP: &str = "00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01";
    run_php(
        &php,
        &ext,
        &cont,
        port,
        &[
            ("HTTP_TRACEPARENT", TP),
            ("REQUEST_METHOD", "GET"),
            ("REQUEST_URI", "/continue"),
        ],
    )?;

    thread::sleep(Duration::from_millis(200));

    let payloads = mock.payloads();
    if payloads.is_empty() {
        return Err("no OTLP payloads received".into());
    }
    let blob = serde_json::to_string(&payloads).map_err(|e| e.to_string())?;
    for need in [
        "rpc.system",
        "LinkServiceProto.LinkService/getTargetLink",
        "php-otel-integration",
    ] {
        if !blob.contains(need) {
            let preview: String = blob.chars().take(2000).collect();
            return Err(format!("missing {need} in {preview}"));
        }
    }
    for need in [
        "cccccccccccccccccccccccccccccccc",
        "dddddddddddddddd",
        "otel.traceparent.continued",
    ] {
        if !blob.contains(need) {
            let preview: String = blob.chars().take(3000).collect();
            return Err(format!("missing continue marker {need} in {preview}"));
        }
    }

    println!("PASS: OTLP received gRPC client span + continued TraceID");
    mock.shutdown();
    Ok(())
}

fn run_php(
    php: &Path,
    ext: &str,
    script: &Path,
    port: u16,
    extra_env: &[(&str, &str)],
) -> Result<(), String> {
    let mut cmd = Command::new(php);
    cmd.args(["-d", ext])
        .arg(script)
        .env("OTEL_ENABLED", "true")
        .env("OTEL_ENDPOINT", format!("127.0.0.1:{port}"))
        .env("OTEL_SAMPLE_RATE", "1.0")
        .env("OTEL_SERVICE_NAME", "php-otel-integration")
        .env("REQUEST_METHOD", "GET")
        .env("REQUEST_URI", "/integration");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {}: {e}", php.display()))?;
    if !status.success() {
        return Err(format!(
            "{} exited with {}",
            script.display(),
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}
