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
import os
import pathlib
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = pathlib.Path(__file__).resolve().parent
PAGE = HERE.parent / "web" / "index.html"
STATUS_PAGE = HERE.parent / "web" / "status.html"
# The RENDERED presets carry real paths; the installed copy still has the
# <QT_MODELS_DIR> placeholder. Either serves for c/parallel/mmproj, but prefer
# the rendered one so the catalog reflects what the server was actually started
# with rather than what the repository says.
PRESETS_CANDIDATES = (
    pathlib.Path("/var/lib/qwen-turing/router-presets.rendered.ini"),
    pathlib.Path("/opt/qwen-turing/etc/router-presets.ini"),
    HERE.parent / "config" / "router-presets.ini",
)


def _presets_text() -> str:
    for c in PRESETS_CANDIDATES:
        try:
            return c.read_text()
        except OSError:
            continue
    return ""

# One timer polls; every handler reads a cache.
#
# This is load-bearing, not an optimisation. nginx runs auth_request against
# /api/queue-headers BEFORE EVERY INFERENCE REQUEST. If that handler polled, each
# completion would wait on three HTTP round-trips and an nvidia-smi fork.
#
# It also protects the arithmetic: QueueWindow.add() must be fed on a fixed
# cadence. Feeding it once per inference request would add irregular samples and
# corrupt the completion counting every ETA depends on.
REFRESH_SECONDS = 1.0
# read_journal FORKS a process, so it must not run on the 1 s tick and must never
# be reachable from /api/queue-headers, which gates every inference request.
JOURNAL_SECONDS = 30.0


def _load_collector():
    """collect-stats.py has a hyphen, so it cannot be imported normally."""
    spec = importlib.util.spec_from_file_location(
        "collect_stats", HERE / "collect-stats.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _load_window():
    spec = importlib.util.spec_from_file_location(
        "queue_window", HERE / "queue_window.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


COLLECT = _load_collector()
# Problem detection lives in its own module so it can be tested without a GPU, a
# journal, or a socket -- and so this file does not grow a second job.
sys.path.insert(0, str(HERE))
import health as HEALTH        # noqa: E402
WINDOW = _load_window().QueueWindow(window_seconds=300.0)

_CACHE: dict = {"llama_up": False, "gpus": [], "queue": {}}
_CACHE_LOCK = threading.Lock()


def _base_url() -> str:
    return Handler.metrics_url.rsplit("/metrics", 1)[0]


def _poll_once() -> dict:
    """The ONLY place that talks to the backend or forks. Never per request."""
    models = COLLECT.read_models(_base_url(), Handler.api_key)
    model = COLLECT.pick_loaded_model(models)
    url = (f"{Handler.metrics_url}?model={model}" if model else Handler.metrics_url)
    metrics = COLLECT.read_metrics(url, Handler.api_key)
    slots = COLLECT.read_slot_total(_base_url(), Handler.api_key, model)
    cards, smi_stderr, smi_failed = COLLECT.read_gpus_with_faults()
    # Liveness from the endpoint this build actually permits: a model list that
    # answers proves the runtime is there, whatever /metrics says about auth.
    summary = COLLECT.summarise(metrics, cards, model, answering=bool(models))
    # Carried under private keys (stripped from every response) so the slow
    # journal timer can turn them into problems without forking nvidia-smi again.
    summary["_smi_stderr"] = smi_stderr
    summary["_smi_failed"] = smi_failed
    q = COLLECT.queue_state(metrics, slots)
    WINDOW.add(time.time(), q["outstanding"])      # exactly once per tick
    q.update({
        "samples": WINDOW.samples,
        "completions": WINDOW.completions,
        "service_rate": WINDOW.service_rate(),
        "mean_service_seconds": WINDOW.mean_service_seconds(),
        "est_wait_seconds": WINDOW.est_wait_seconds(q["queued"]),
    })
    summary["queue"] = q
    summary["catalog"] = COLLECT.model_catalog(models, _presets_text())
    summary["_metrics"] = metrics
    summary["_models"] = models
    return summary


def _gpu_gate(snap: dict) -> dict:
    """A GPU verdict the gateway can act on, refreshed on the FAST tick.

    Published here rather than computed in the gateway because the gateway runs
    with PrivateDevices=true -- /dev/nvidia* is hidden from it, so nvidia-smi
    there runs and honestly reports no cards. Asking it to probe GPUs meant
    either weakening the sandbox of the internet-facing process or believing a
    false negative; this is neither. (Measured 2026-08-24: the first version of
    that gate 503'd a healthy node for exactly this reason.)

    NOT derived from `problems`: that list is merged from several sources on a
    30 s timer and includes entries about OTHER servers, so a remote node's
    fault could refuse local inference. This uses only the GPU detectors, on
    this node's own fresh nvidia-smi data.
    """
    try:
        expect = int(os.environ.get("QT_EXPECT_DEVICES", "0"))
    except ValueError:
        expect = 0
    cards = snap.get("gpus") or []
    failed = bool(snap.get("_smi_failed"))
    problems = HEALTH.gpu_problems(snap.get("_smi_stderr") or "", cards, failed)
    down = [p for p in problems if p.get("severity") == HEALTH.DOWN]
    if down:
        return {"ok": False, "reason": down[0]["text"]}
    # "could not run nvidia-smi" is WARNING, not DOWN: a telemetry outage must
    # not become an inference outage.
    if not failed and expect > 0 and len(cards) < expect:
        return {"ok": False,
                "reason": f"only {len(cards)} of {expect} GPUs are visible "
                          f"to this node"}
    return {"ok": True, "reason": ""}


def _refresher() -> None:
    while True:
        try:
            snap = _poll_once()
            with _CACHE_LOCK:
                # Preserve whatever a slower timer last wrote.
                snap["config_health"] = _CACHE.get("config_health")
                # Both written by the slow journal timer; the fast poll must not
                # blank them between its ticks.
                snap["problems"] = _CACHE.get("problems") or []
                # Fast tick: the gate must see a card vanish within a second,
                # not on the 30 s journal timer.
                snap["gpu_gate"] = _gpu_gate(snap)
                _CACHE.clear()
                _CACHE.update(snap)
        except Exception:
            # A refresh failure must never kill the timer: a dead thread would
            # freeze the cache at a stale value and look like a quiet node.
            pass
        time.sleep(REFRESH_SECONDS)


def _journal_refresher() -> None:
    """Slow timer: the journal scrape forks, so it gets its own cadence.

    The cheap cache hit-rate still updates every second via _poll_once's metrics;
    only the event scrape is throttled.
    """
    while True:
        try:
            text, readable = COLLECT.read_journal()
            events = COLLECT.parse_journal_events(text) if readable else None
            with _CACHE_LOCK:
                metrics = _CACHE.get("_metrics") or {}
                # What is WRONG, on the slow timer beside the journal it reads.
                # The runtime faults that matter -- a CUDA failure, an Xid -- are
                # in the log and nowhere else: a process that cannot use a GPU
                # still answers /metrics, which is how this node once reported
                # seven healthy models it could not serve.
                _CACHE["problems"] = (
                    HEALTH.runtime_problems(bool(_CACHE.get("llama_up")),
                                            text, readable)
                    + HEALTH.gpu_problems(_CACHE.get("_smi_stderr") or "",
                                          _CACHE.get("gpus") or [],
                                          bool(_CACHE.get("_smi_failed"))))
                _CACHE["config_health"] = {
                    "journal_readable": readable,
                    # None, NOT an empty structure: "unreadable" and "no
                    # evictions" must never render the same, because no evictions
                    # is exactly the good news an operator would act on.
                    "events": events,
                    "cache": COLLECT.cache_health(metrics),
                    "window": "since -2h",
                }
        except Exception:
            pass
        time.sleep(JOURNAL_SECONDS)


def snapshot() -> dict:
    with _CACHE_LOCK:
        return dict(_CACHE)


class Handler(BaseHTTPRequestHandler):
    server_version = "qwen-turing-dashboard"
    api_key: str | None = None
    metrics_url: str = "http://127.0.0.1:8080/metrics"
    # Set for the duration of one HEAD. A class attribute, not per-request
    # state, because do_HEAD() reuses do_GET() wholesale.
    _head = False

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
        # A HEAD gets the same headers and none of the bytes. The body is still
        # built, so Content-Length stays the length a GET would report.
        if not self._head:
            self.wfile.write(body)

    def do_HEAD(self) -> None:  # noqa: N802
        """A HEAD is a GET with the body dropped, so it runs the same code.

        The alternative -- letting BaseHTTPRequestHandler answer 501 -- is what
        this served until now, so every monitor probing the page or /api/queue
        with a HEAD saw a broken node.

        Cleared in a `finally`: one handler instance serves every request on a
        keep-alive connection, and a stuck flag would silence the next real GET.
        """
        self._head = True
        try:
            self.do_GET()
        finally:
            self._head = False

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
            full = {k: v for k, v in snapshot().items() if not k.startswith("_")}
            self._send(200, json.dumps(full).encode(), "application/json")
            return

        if path == "/api/queue":                  # PUBLIC -- load only
            payload = COLLECT.public_payload(snapshot())
            self._send(200, json.dumps(payload).encode(), "application/json")
            return

        if path == "/status":                     # PUBLIC page -- load only
            try:
                self._send(200, STATUS_PAGE.read_bytes(), "text/html; charset=utf-8")
            except OSError:
                self._send(500, b"status page missing", "text/plain")
            return

        if path == "/v1/models":                  # PUBLIC, SANITISED
            # llama.cpp's own /v1/models leaks the child argv including the API
            # key's file location. This serves the cleaned list instead.
            up = snapshot().get("_models") or {}
            self._send(200, json.dumps(COLLECT.sanitise_models(up)).encode(),
                       "application/json")
            return

        if path == "/api/queue-headers":          # for nginx auth_request
            # ALWAYS 204, never a body, and NEVER any I/O -- this gates every
            # inference request. nginx treats a non-2xx auth_request as a
            # rejection, so an endpoint that could be slow or fail would turn a
            # busy dashboard into a refusal of service.
            try:
                q = snapshot().get("queue") or {}
            except Exception:
                q = {}

            def h(v):
                return "" if v is None else str(v)

            self.send_response(204)
            self.send_header("X-Queue-Slots", h(q.get("slots")))
            self.send_header("X-Queue-Processing", h(q.get("processing")))
            self.send_header("X-Queue-Depth", h(q.get("queued")))
            self.send_header("X-Queue-Fullness",
                             h(round(q["fullness"], 3)
                               if q.get("fullness") is not None else None))
            self.send_header("X-Queue-Est-Wait-Seconds",
                             h(round(q["est_wait_seconds"])
                               if q.get("est_wait_seconds") is not None else None))
            self.send_header("Content-Length", "0")
            self.end_headers()
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
            # FIRST LINE only. A key file may legitimately hold several keys --
            # llama.cpp reads one per line -- and read().strip() would join them
            # into a value that matches nothing, rejecting every caller. This was
            # a live time bomb: the file gained a second line during the gateway
            # cutover and only an un-restarted process was still working.
            Handler.api_key = (open(args.api_key_file).readline() or "").strip() or None
        except OSError:
            print(f"cannot read {args.api_key_file}", file=sys.stderr)
            return 78
    Handler.metrics_url = args.metrics_url

    threading.Thread(target=_refresher, daemon=True).start()
    threading.Thread(target=_journal_refresher, daemon=True).start()
    # Give the first tick a moment so the very first request sees real numbers
    # rather than an empty cache.
    time.sleep(1.2)

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
