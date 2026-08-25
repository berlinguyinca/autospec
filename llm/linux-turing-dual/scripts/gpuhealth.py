#!/usr/bin/env python3
"""Is this node's GPU stack usable RIGHT NOW? (gateway side)

Startup is covered elsewhere: vram-guard.sh refuses a missing driver or a
missing card, and serve-router.sh pins --device CUDA0,CUDA1 so llama.cpp exits 1
instead of falling back to the CPU. Measured on this host 2026-08-24:

    --device CUDA9           -> "invalid device: CUDA9", exit 1
    CUDA_VISIBLE_DEVICES=""  -> "failed to initialize CUDA", exit 0, SERVES ON CPU

None of that helps once the node is already running -- a card that falls off the
bus at 14:00 leaves a process that started correctly hours earlier. Hence a
runtime gate.

WHY THIS ASKS THE DASHBOARD INSTEAD OF LOOKING ITSELF. The gateway unit sets
PrivateDevices=true, so /dev/nvidia* is hidden from it: nvidia-smi runs there,
cannot reach the driver, and honestly reports no cards. The first version of
this module probed directly and 503'd a completely healthy node for exactly that
reason. The alternatives were weakening the sandbox of the internet-facing
process or believing a false negative; asking the component that legitimately
owns GPU access is neither. The dashboard publishes `gpu_gate` on its 1 s tick.

FAIL OPEN. Any failure to obtain a verdict -- dashboard down, restarting, slow,
or too old to have the field -- means SERVE. A gate that cannot reach its
telemetry and therefore refuses inference has converted a monitoring outage into
an outage, which is worse than the fault it exists to catch. nginx makes the
same trade in this repo: fail open for telemetry, fail closed for auth.
"""
from __future__ import annotations

import http.client
import json
import os
import threading
import time

TTL = float(os.environ.get("QT_GPU_HEALTH_TTL", "3"))
TIMEOUT = float(os.environ.get("QT_GPU_HEALTH_TIMEOUT", "2"))

_LOCK = threading.Lock()
_CACHE: tuple[float, bool, str] | None = None   # (expires, ok, reason)

# /api/stats is authenticated. Injected by the gateway rather than re-read from
# disk here: the key arrives as a systemd credential the gateway already holds,
# and a second reader would be a second thing to keep in step.
_PORT: int | None = None
_KEY: str = ""


def configure(port: int, key: str) -> None:
    global _PORT, _KEY
    _PORT, _KEY = int(port), key or ""
    invalidate()


def _dash_port() -> int:
    if _PORT is not None:
        return _PORT
    try:
        return int(os.environ.get("QT_DASH_PORT_LOCAL", "8081"))
    except ValueError:
        return 8081


def _fetch() -> dict | None:
    """The dashboard's stats, or None if it could not be obtained."""
    conn = None
    try:
        conn = http.client.HTTPConnection("127.0.0.1", _dash_port(),
                                          timeout=TIMEOUT)
        headers = {"Authorization": f"Bearer {_KEY}"} if _KEY else {}
        conn.request("GET", "/api/stats", headers=headers)
        r = conn.getresponse()
        if r.status != 200:
            # 401 lands here when the key is missing or stale. That is "no
            # verdict", not "GPUs are down", and the caller fails open.
            return None
        return json.loads(r.read().decode("utf-8", "replace"))
    except Exception:
        return None
    finally:
        if conn is not None:
            try:
                conn.close()
            except Exception:
                pass


def _probe() -> tuple[bool, str]:
    data = _fetch()
    if not isinstance(data, dict):
        return True, ""                       # fail open
    gate = data.get("gpu_gate")
    if not isinstance(gate, dict) or "ok" not in gate:
        return True, ""                       # older dashboard: fail open
    if gate.get("ok"):
        return True, ""
    return False, str(gate.get("reason") or "the GPUs are not usable")


def verdict(now: float | None = None) -> tuple[bool, str]:
    """(ok, reason). reason is '' when ok."""
    global _CACHE
    now = time.time() if now is None else now
    with _LOCK:
        if _CACHE is not None and _CACHE[0] > now:
            return _CACHE[1], _CACHE[2]
    ok, reason = _probe()
    with _LOCK:
        _CACHE = (now + TTL, ok, reason)
    return ok, reason


def invalidate() -> None:
    global _CACHE
    with _LOCK:
        _CACHE = None
