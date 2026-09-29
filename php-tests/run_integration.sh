#!/usr/bin/env bash
# Run PHP integration against a mock OTLP receiver (expects otel_auto.so built for this PHP).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SO="${1:-$ROOT/otel_auto.so}"
PORT="${OTLP_PORT:-14318}"
OUT="${OTLP_OUT:-/tmp/otel-auto-otlp-spans.json}"

if [[ ! -f "$SO" ]]; then
  echo "missing $SO — build with Dockerfile.build first" >&2
  exit 1
fi

python3 "$ROOT/php-tests/mock_otlp_receiver.py" "$PORT" "$OUT" &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
sleep 0.3

export OTEL_ENABLED=true
export OTEL_ENDPOINT="127.0.0.1:${PORT}"
export OTEL_SAMPLE_RATE=1.0
export OTEL_SERVICE_NAME=php-otel-integration
export REQUEST_METHOD=GET
export REQUEST_URI=/integration

php -d "extension=$SO" "$ROOT/php-tests/smoke.php"
php -d "extension=$SO" "$ROOT/php-tests/integration_hooks.php"

# Give exporter a moment (flush on RSHUTDOWN already happened)
sleep 0.2
python3 - <<PY
import json, sys
from pathlib import Path
raw = Path("$OUT").read_text()
data = json.loads(raw)
if not data:
    print("FAIL: no OTLP payloads received", file=sys.stderr)
    sys.exit(1)
blob = json.dumps(data)
need = ["rpc.system", "LinkServiceProto.LinkService/getTargetLink", "php-otel-integration"]
for n in need:
    if n not in blob:
        print(f"FAIL: missing {n} in {blob[:2000]}", file=sys.stderr)
        sys.exit(1)
print("PASS: OTLP received gRPC client span")
PY
