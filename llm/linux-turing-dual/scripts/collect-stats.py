#!/usr/bin/env python3
"""Collect node statistics: llama.cpp /metrics plus per-card nvidia-smi.

Standard library only, and deliberately so. llama.cpp already publishes prompt
tokens, generated tokens, throughput and KV-pool usage, and nvidia-smi reports
per-card utilisation -- so the whole stats story is this file plus one HTML page,
rather than Prometheus, Grafana and a GPU exporter in containers.

Both parsers degrade to empty rather than raising. The model is unavailable for
several seconds on every switch, and a stats page that 500s during a reload is
worse than one that shows a gap.

Usage:
    collect-stats.py [--metrics-url URL] [--api-key-file FILE]
prints one JSON object.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

NVIDIA_QUERY = ("index,name,utilization.gpu,memory.used,"
                "memory.total,temperature.gpu,power.draw")


def parse_prometheus(text: str) -> dict[str, float]:
    """Prometheus exposition text -> {name: value}.

    Comments and HELP/TYPE lines are not metrics. A value that will not parse is
    skipped rather than aborting the whole read.
    """
    out: dict[str, float] = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        try:
            out[parts[0]] = float(parts[-1])
        except ValueError:
            continue
    return out


def _num(value: str, cast=int):
    """nvidia-smi prints [N/A] for sensors a card does not expose."""
    try:
        return cast(float(value))
    except ValueError:
        return None


def parse_nvidia_csv(text: str) -> list[dict]:
    """One dict per card. An empty read is an empty list, never an exception.

    Every card is returned. Reading only the first row is the bug that ran
    through this entire toolkit before a second card existed to expose it.
    """
    gpus: list[dict] = []
    for line in text.splitlines():
        if not line.strip():
            continue
        f = [c.strip() for c in line.split(",")]
        if len(f) < 7:
            continue
        gpus.append({
            "index": _num(f[0]),
            "name": f[1],
            "util_pct": _num(f[2]),
            "mem_used_mib": _num(f[3]),
            "mem_total_mib": _num(f[4]),
            "temp_c": _num(f[5]),
            "power_w": _num(f[6], float),
        })
    return gpus


def summarise(metrics: dict[str, float], gpus: list[dict],
              model: str | None = None) -> dict:
    """Join the two sources into what the page renders."""
    def m(name: str, default: float = 0.0) -> float:
        return metrics.get(f"llamacpp:{name}", default)

    return {
        "llama_up": bool(metrics),
        "model": model,
        "prompt_tokens_total": int(m("prompt_tokens_total")),
        "generated_tokens_total": int(m("tokens_predicted_total")),
        "tokens_per_second": m("predicted_tokens_seconds"),
        "kv_cache_usage_ratio": m("kv_cache_usage_ratio"),
        "requests_processing": int(m("requests_processing")),
        "requests_deferred": int(m("requests_deferred")),
        "gpu_count": len(gpus),
        # Summed across cards. On this node the pair is the unit of capacity.
        "gpu_total_mem_mib": sum(g["mem_total_mib"] or 0 for g in gpus),
        "gpu_used_mem_mib": sum(g["mem_used_mib"] or 0 for g in gpus),
        "gpus": gpus,
    }


def pick_loaded_model(models: dict) -> str | None:
    """Which model id is currently resident.

    In router mode /metrics REQUIRES ?model=<id> and returns HTTP 400 without it,
    so there is no single endpoint that just reports the node's throughput. The
    router publishes each preset with a status.value of "loaded" or "unloaded";
    with --models-max 1 at most one is loaded at a time.
    """
    for m in (models or {}).get("data", []):
        if (m.get("status") or {}).get("value") == "loaded":
            return m.get("id")
    return None


def read_models(base: str, api_key: str | None, timeout: float = 4.0) -> dict:
    req = urllib.request.Request(base.rstrip("/") + "/v1/models")
    if api_key:
        req.add_header("Authorization", f"Bearer {api_key}")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except (urllib.error.URLError, OSError, TimeoutError, ValueError):
        return {}


def read_metrics(url: str, api_key: str | None, timeout: float = 4.0) -> dict[str, float]:
    req = urllib.request.Request(url)
    if api_key:
        req.add_header("Authorization", f"Bearer {api_key}")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return parse_prometheus(r.read().decode("utf-8", "replace"))
    except (urllib.error.URLError, OSError, TimeoutError):
        # The unit may be down or mid-reload. That is a gap, not an error.
        return {}


def read_gpus(timeout: float = 6.0) -> list[dict]:
    try:
        out = subprocess.run(
            ["nvidia-smi", f"--query-gpu={NVIDIA_QUERY}",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=timeout, check=True).stdout
        return parse_nvidia_csv(out)
    except (OSError, subprocess.SubprocessError):
        return []


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--metrics-url", default="http://127.0.0.1:8080/metrics")
    ap.add_argument("--api-key-file")
    args = ap.parse_args()

    key = None
    if args.api_key_file:
        try:
            key = open(args.api_key_file).read().strip()
        except OSError:
            key = None

    # Two steps, because router mode has no unqualified /metrics: discover the
    # resident model, then ask for that model's metrics.
    base = args.metrics_url.rsplit("/metrics", 1)[0]
    model = pick_loaded_model(read_models(base, key))
    url = f"{args.metrics_url}?model={model}" if model else args.metrics_url
    payload = summarise(read_metrics(url, key), read_gpus(), model)
    json.dump(payload, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())


# --- queue depth -----------------------------------------------------------
def read_slot_total(base: str, api_key: str | None, model: str | None,
                    timeout: float = 4.0) -> int:
    """total_slots for the resident model. 0 when unknown, never a guess."""
    if not model:
        return 0
    req = urllib.request.Request(
        base.rstrip("/") + "/props?model=" + urllib.parse.quote(model))
    if api_key:
        req.add_header("Authorization", f"Bearer {api_key}")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            d = json.loads(r.read().decode("utf-8", "replace"))
        return int(d.get("total_slots") or 0)
    except (urllib.error.URLError, OSError, TimeoutError, ValueError):
        return 0


def queue_state(metrics: dict, slots_total: int) -> dict:
    """Queue depth and capacity. Missing metrics read as zero, not unknown."""
    def m(name: str) -> int:
        return int(metrics.get("llamacpp:" + name, 0) or 0)

    processing = m("requests_processing")
    queued = m("requests_deferred")
    slots = int(slots_total or 0)
    outstanding = processing + queued
    return {
        "slots": slots,
        "processing": processing,
        "queued": queued,
        "outstanding": outstanding,
        # May exceed 1.0 -- that is the point of showing it. None when the slot
        # count is unknown, because 0 would read as "empty".
        "fullness": (outstanding / slots) if slots > 0 else None,
    }


# --- the unauthenticated surface -------------------------------------------
# An explicit allow-list, not a deny-list. /api/queue and /status are reachable
# without a key on a single public port, so a field added to the full payload
# later must not be able to appear here by default.
PUBLIC_FIELDS = frozenset({
    "slots", "processing", "queued", "fullness", "est_wait_seconds",
    "service_rate", "mean_service_seconds", "samples", "completions",
    "model_loaded",
})


def public_payload(full: dict) -> dict:
    """Project the full summary onto the public allow-list.

    Built by iterating PUBLIC_FIELDS rather than by deleting private keys, so the
    failure mode of a forgotten field is a MISSING value, never a leak.
    """
    q = full.get("queue") or {}
    src = {
        "slots": q.get("slots", 0),
        "processing": q.get("processing", 0),
        "queued": q.get("queued", 0),
        "fullness": q.get("fullness"),
        "est_wait_seconds": q.get("est_wait_seconds"),
        "service_rate": q.get("service_rate"),
        "mean_service_seconds": q.get("mean_service_seconds"),
        "samples": q.get("samples", 0),
        "completions": q.get("completions", 0),
        # A boolean, never the model name: which model is loaded is node
        # configuration, not load.
        "model_loaded": bool(full.get("llama_up")),
    }
    return {k: src[k] for k in sorted(PUBLIC_FIELDS)}


_MODEL_FIELDS = ("id", "aliases", "object", "owned_by", "created")


def sanitise_models(upstream: dict) -> dict:
    """Strip llama.cpp's /v1/models down to what a client needs.

    Upstream publishes each child instance's full argv WITHOUT authentication,
    including the API key's file location. Clients genuinely need this endpoint
    for discovery, so it is cleaned rather than blocked -- and `status` is dropped
    wholesale rather than filtered, because that is where argv lives.
    """
    out = []
    data = (upstream or {}).get("data")
    if isinstance(data, list):
        for m in data:
            if isinstance(m, dict):
                out.append({k: m[k] for k in _MODEL_FIELDS if k in m})
    return {"object": "list", "data": out}
