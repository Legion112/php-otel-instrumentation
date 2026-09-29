# Alanbase PHP OTEL auto-instrumentation

Rust PHP 8.2 extension that automatically creates OpenTelemetry spans for:

- HTTP / CLI request roots
- `Grpc\BaseStub::_simpleRequest` (client) and RoadRunner GRPC `Invoker::invoke` (server)
- PDO (`query` / `exec` / `PDOStatement::execute`)
- Redis / RedisCluster common commands
- `curl_exec`

Exports OTLP/HTTP JSON to the existing stack (`OTEL_ENDPOINT`, default `alloy:4318`).

**No application code changes.** Install only via the [image-replacement](https://gitlab.dev-alan.com/legion/image-replacement) layer onto `php-fpm-base:php-8.2` and `php-grpc:php-8.2-rr-v1`.

## Env (already in infra `common.env`)

| Variable | Meaning |
|----------|---------|
| `OTEL_ENABLED` | `true`/`false` (default true) |
| `OTEL_ENDPOINT` | `alloy:4318` or `host.docker.internal:4318` |
| `OTEL_SAMPLE_RATE` | `0.0`–`1.0` |
| `OTEL_SERVICE_NAME` | optional; else `HOSTNAME` / `php` |

## Build `.so` (PHP 8.2)

```bash
docker build -f Dockerfile.build --target builder -t alanbase-otel-build .
docker create --name otel-so alanbase-otel-build
docker cp otel-so:/tmp/alanbase_otel.so ./alanbase_otel.so
docker rm otel-so
```

## Unit tests (no PHP)

```bash
cargo test -p alanbase-otel-core
```

## Layout

- `crates/core` — config, gRPC path parse, span model, OTLP JSON export
- `crates/extension` — ext-php-rs observer + request lifecycle
