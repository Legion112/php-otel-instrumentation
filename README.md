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

## Environment

| Variable | Meaning |
|----------|---------|
| `OTEL_ENABLED` | `true`/`false` (default true) |
| `OTEL_ENDPOINT` | host:port or URL (e.g. `alloy:4318`) |
| `OTEL_SAMPLE_RATE` | `0.0`–`1.0` |
| `OTEL_SERVICE_NAME` | optional override for `service.name` |
| *(fallback)* | `APP_NAME` → `HOSTNAME` → `php` |

## Build `.so` (PHP 8.2 NTS)

```bash
docker build -f Dockerfile.build --target builder -t otel-auto-build .
docker create --name otel-so otel-auto-build
docker cp otel-so:/tmp/otel_auto.so ./otel_auto.so
docker rm otel-so
```

Enable with:

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
