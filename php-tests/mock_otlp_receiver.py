#!/usr/bin/env python3
"""Minimal OTLP/HTTP JSON receiver for PHP integration tests."""
from __future__ import annotations

import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


class Handler(BaseHTTPRequestHandler):
    store: list = []
    path_file: Path | None = None

    def log_message(self, fmt, *args):  # noqa: N802
        return

    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        try:
            payload = json.loads(body.decode("utf-8"))
        except Exception:
            payload = {"raw": body.decode("utf-8", errors="replace")}
        Handler.store.append(payload)
        if Handler.path_file is not None:
            Handler.path_file.write_text(json.dumps(Handler.store, indent=2))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b"{}")


def main() -> int:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 4318
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("/tmp/otlp-spans.json")
    Handler.path_file = out
    Handler.store = []
    out.write_text("[]")
    server = HTTPServer(("0.0.0.0", port), Handler)
    print(f"otlp-mock listening on :{port} -> {out}", flush=True)
    t = threading.Thread(target=server.serve_forever, daemon=True)
    t.start()
    try:
        t.join()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
