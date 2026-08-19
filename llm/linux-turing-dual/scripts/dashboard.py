#!/usr/bin/env python3
"""Serve the node stats page and its JSON endpoint.

One listener, one page, the standard library. It reuses the inference API key
rather than growing an auth story of its own -- if this file ever needs its own
auth framework or a second port, it has regrown exactly the complexity that was
deliberately removed when Prometheus and Grafana were dropped.

    dashboard.py --host H --port P --metrics-url URL --api-key-file FILE
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = pathlib.Path(__file__).resolve().parent
PAGE = HERE.parent / "web" / "index.html"


def _load_collector():
    """collect-stats.py has a hyphen, so it cannot be imported normally."""
    spec = importlib.util.spec_from_file_location(
        "collect_stats", HERE / "collect-stats.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


COLLECT = _load_collector()


class Handler(BaseHTTPRequestHandler):
    server_version = "qwen-turing-dashboard"
    api_key: str | None = None
    metrics_url: str = "http://127.0.0.1:8080/metrics"

    def _authorised(self) -> bool:
        if not self.api_key:
            return True
        sent = self.headers.get("Authorization", "")
        if sent.startswith("Bearer "):
            sent = sent[7:]
        # Constant-time compare: this endpoint is reachable from the network.
        import hmac
        return hmac.compare_digest(sent, self.api_key)

    def _send(self, code: int, body: bytes, ctype: str) -> None:
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0]

        if path in ("/", "/index.html"):
            # The page itself is public; every NUMBER on it requires the key.
            try:
                self._send(200, PAGE.read_bytes(), "text/html; charset=utf-8")
            except OSError:
                self._send(500, b"page missing", "text/plain")
            return

        if path == "/api/stats":
            if not self._authorised():
                self._send(401, b'{"error":"unauthorised"}', "application/json")
                return
            # Router mode has no unqualified /metrics -- it returns HTTP 400
            # without ?model=<id>. So discover the resident model first.
            base = self.metrics_url.rsplit("/metrics", 1)[0]
            model = COLLECT.pick_loaded_model(
                COLLECT.read_models(base, self.api_key))
            url = (f"{self.metrics_url}?model={model}" if model
                   else self.metrics_url)
            payload = COLLECT.summarise(
                COLLECT.read_metrics(url, self.api_key),
                COLLECT.read_gpus(), model)
            self._send(200, json.dumps(payload).encode(), "application/json")
            return

        self._send(404, b"not found", "text/plain")

    def log_message(self, fmt, *args):
        # Journald already timestamps; keep one line per request.
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8081)
    ap.add_argument("--metrics-url", default="http://127.0.0.1:8080/metrics")
    ap.add_argument("--api-key-file")
    args = ap.parse_args()

    if args.api_key_file:
        try:
            Handler.api_key = open(args.api_key_file).read().strip() or None
        except OSError:
            print(f"cannot read {args.api_key_file}", file=sys.stderr)
            return 78
    Handler.metrics_url = args.metrics_url

    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"dashboard on http://{args.host}:{args.port} "
          f"(metrics from {args.metrics_url})", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
