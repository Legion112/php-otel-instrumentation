# PHP OTEL auto-instrumentation

Self-contained Rust PHP extension that automatically creates OpenTelemetry spans with **no application code changes**.

Instruments:

- HTTP / CLI request roots
- `Grpc\BaseStub::_simpleRequest` (client) and RoadRunner GRPC `Invoker::invoke` (server)
- PDO (`query` / `exec` / `PDOStatement::execute`)
- Redis / RedisCluster common commands
- `curl_exec`

Exports OTLP/HTTP JSON (`OTEL_ENDPOINT`, default `alloy:4318`).

PHP module name: **`otel_auto`** (loads as `otel_auto.so`).

Supported runtimes: **PHP 8.2 / 8.5 NTS**, **linux x86_64 / aarch64**.

## Download (prebuilt)

Do **not** use pecl.php.net — this is a Rust/`ext-php-rs` extension, not a classic C PECL package. Install path is **[GitHub Releases](https://github.com/Legion112/php-otel-instrumentation/releases)**.

Pick the asset that matches `php -v` and `uname -m`:

| PHP | Arch | Asset |
|-----|------|-------|
| 8.2 | x86_64 | `otel_auto-php8.2-linux-x86_64.so` |
| 8.2 | aarch64 | `otel_auto-php8.2-linux-aarch64.so` |
| 8.5 | x86_64 | `otel_auto-php8.5-linux-x86_64.so` |
| 8.5 | aarch64 | `otel_auto-php8.5-linux-aarch64.so` |

```bash
# example: PHP 8.5 on x86_64
curl -LO https://github.com/Legion112/php-otel-instrumentation/releases/latest/download/otel_auto-php8.5-linux-x86_64.so
# copy into extension_dir and enable:
# extension=otel_auto.so
```

ABI must match: an 8.2-built `.so` will not load on PHP 8.5 (and vice versa). Same for arch.

## Environment

| Variable | Meaning |
|----------|---------|
| `OTEL_ENABLED` | `true`/`false` (default true) |
| `OTEL_ENDPOINT` | host:port or URL (e.g. `alloy:4318`) |
| `OTEL_SAMPLE_RATE` | `0.0`–`1.0` |
| `OTEL_SERVICE_NAME` | optional override for `service.name` |
| *(fallback)* | `APP_NAME` → `HOSTNAME` → `php` |

## Build `.so` from source (PHP 8.2 / 8.5 NTS)

```bash
# default PHP_VERSION=8.5; use 8.2 for the other ABI
docker build -f Dockerfile.build --target builder \
  --build-arg PHP_VERSION=8.5 \
  -t otel-auto-build .
docker create --name otel-so otel-auto-build
docker cp otel-so:/tmp/otel_auto.so ./otel_auto.so
docker rm otel-so
```

Build on the same CPU arch you will run on (or use CI Release assets). Enable with:

```ini
extension=otel_auto.so
```

## Unit tests (no PHP)

```bash
cargo test -p otel-auto-core
```

## PHP integration (needs `.so` + `php` matching that build)

```bash
# after Dockerfile.build extract of otel_auto.so
cargo run -p xtask -- php-integration
# or: cargo run -p xtask -- php-integration --so ./otel_auto.so --php php
```

## Layout

- `crates/core` — config, gRPC path parse, span model, OTLP JSON export
- `crates/extension` — ext-php-rs observer + request lifecycle
- `crates/xtask` — Rust mock OTLP + PHP integration runner
- `php-tests/` — PHP fixtures (`smoke.php`, `integration_hooks.php`)

## License

MIT
