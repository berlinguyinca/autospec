# Queue Visibility, Header Proxy and 100k Tier — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> ## ✅ EXECUTED — 2026-08-19
>
> All eleven tasks were carried out and the node is live. **The authoritative
> record of what actually happened is
> [`llm/linux-turing-dual/docs/measured-ceilings.md`](../../../llm/linux-turing-dual/docs/measured-ceilings.md)**,
> not this file — it carries the measured numbers, and this file carries only the
> intent.
>
> **Where this plan diverged from reality**, so a reader does not follow stale
> instructions:
>
> | plan said | reality |
> |---|---|
> | `check_presets.py --config <file>` | takes a **positional** argument, no flag |
> | report `p50` / `p95` response times | **impossible** without a per-request counter; replaced by `mean_service_seconds` and `service_rate`, divided by *busy* seconds |
> | `_snapshot()` computed per request | renamed `snapshot()` and reads a **cached** value; one timer polls, because `auth_request` gates every inference request |
> | `requests_deferred` might be unusable | **usable** — peaks at 4 under a 6-request burst; the `/slots` fallback was never needed |
> | 100k prefill ≈ 210 s at 475 tok/s | **594–637 tok/s** single-session; the 475 figure came from a *concurrent* run |
> | `cache-reuse` makes the 100k tier usable | **silently discarded** on this model; ordinary prefix caching does the work, and one slot is what secures it |
> | 100k as a tier alias | a **dedicated `parallel = 1` preset**, because two slots give a cache hit only every other turn |
> | 9B served on plain `Q4_K_M` | moved to `UD-Q4_K_XL`; the 27B was already Dynamic v3.0 |
>
> Steps are ticked to show they ran, not to suggest the text is still accurate.

**Goal:** Make the node's queue visible (depth, capacity, measured wait), serve everything through nginx on port 80, and add a 100k context tier.

**Architecture:** llama.cpp already queues in slot order, so this adds observation rather than scheduling. nginx becomes the only public listener on `:80` (plus `:8080` for compatibility); llama.cpp retreats to `127.0.0.1:8090` and the dashboard to `127.0.0.1:8081`. The dashboard's collector counts completions from decreases in `processing + queued` over a rolling window and derives a measured service rate; nginx injects `X-Queue-*` headers via an `auth_request` subrequest to the dashboard. The 100k tier is a pool increase plus tier aliases, not a new preset.

**Tech Stack:** Python 3.12 stdlib (no new deps), nginx, bash, systemd, ufw, pytest.

**Spec:** [`docs/superpowers/specs/2026-08-19-queue-visibility-design.md`](../specs/2026-08-19-queue-visibility-design.md)

## Global Constraints

- **The repository is public.** No hostname, IP, subnet or campus identifier under `llm/` or `docs/memory/`. `tests/test_structural.sh` check 7 fails on any literal IPv4 except RFC 5737 documentation ranges, `0.0.0.0` and loopback. Real values live in `/etc/qwen-turing/site.conf`.
- **Never configure a number you have not verified with a real request.**
- **No new Python dependencies.** Standard library only — the dashboard was deliberately built without Prometheus/Grafana.
- **`est_wait_seconds` is `null` until completions have been observed.** Never `0`. The sample count travels with the estimate.
- **No percentiles.** There is no per-request counter; only `mean_service_seconds` and `service_rate`, both divided by **busy** seconds, not wall-clock.
- **Public payload allow-list** for `/api/queue` and `/status`: `slots`, `processing`, `queued`, `fullness`, `est_wait_seconds`, `service_rate`, `mean_service_seconds`, `samples`, `completions`, `model_loaded`. Nothing else, ever.
- **`ProtectSystem=full`, never `strict`.** Never `PrivateDevices=true`, `DevicePolicy=closed`, or `MemoryDenyWriteExecute=`. Each breaks CUDA. `test_structural.sh` check 5 enforces this.
- **`proxy_read_timeout 900s`**, `proxy_buffering off`, `proxy_request_buffering off`. A 100k prompt needs ~210 s before its first token and bodies reach 230 KB.
- **Cross-slot KV reuse stays `0.0`** (`slot-prompt-similarity`) — the path commit `1cd8c1f4` disabled for crashing the model child.
- **`cache-reuse` is REMOVED, not tuned.** The node logs `cache_reuse is not supported by this context, it will be disabled` — a hybrid model's recurrent layers cannot be partially rewound. Leaving it in the presets makes a discarded option look effective.
- **Prefix caching is real and worth 9x**, but only when a request lands on the slot holding its prefix. Measured with a 20k shared prefix: 19.8 s / 20.4 s / **2.3 s**, the miss being round-robin assignment to a cold slot.
- **`auth_request` must never reject an inference request.** The header endpoint always returns 204.
- **Pool is `c = 102400`.** Tier aliases: `-100k` (1 seat), `-50k` (2 seats), `-40k` (2 seats). `check_presets.py` must pass.
- **Commit shaping:** every source commit needs its own doc touch; keep commits under ~400 lines.

---

## File Structure

**New:**

| path | responsibility |
|---|---|
| `llm/linux-turing-dual/scripts/queue_window.py` | rolling-window completion counting and service-rate arithmetic; pure, no I/O |
| `llm/linux-turing-dual/nginx/qwen-turing.conf` | the only public listener; routing, header injection, TLS scaffolding |
| `llm/linux-turing-dual/web/status.html` | public load page, no key, no prompts |
| `llm/linux-turing-dual/tests/test_unit_queue_window.py` | window arithmetic incl. zero-sample and idle cases |
| `llm/linux-turing-dual/tests/test_unit_public_payload.py` | allow-list and `/v1/models` sanitising |

**Modified:**

| path | change |
|---|---|
| `scripts/collect-stats.py` | queue fields, `queue_state()`, `public_payload()`, `sanitise_models()` |
| `scripts/dashboard.py` | `/api/queue`, `/status`, `/api/queue-headers`, `/v1/models`; window wiring |
| `scripts/dashboard-run.sh` | bind loopback |
| `scripts/serve-router.sh` | bind `127.0.0.1:8090` |
| `config/common.conf` | `QT_LLAMA_PORT`, pool 102400, nginx timeouts |
| `config/router-presets.ini` | `c = 102400`, three tier aliases |
| `config/site.conf.example` | `QT_PUBLIC_PORT`, loopback backends |
| `scripts/site.sh` | new required vars |
| `scripts/install-node.sh` | install nginx config, validate it |
| `web/index.html` | queue panel + connection examples |
| `tests/test_structural.sh` | nginx-config checks |
| `ops/firewall.sh` | port 80 |
| `docs/measured-ceilings.md` | 100k measurements, queue findings |
| `README.md` (node) | one-URL usage, nginx as SPOF |

---

### Task 1: Pin down what `requests_deferred` actually means

**Blocking.** A six-request burst never showed it above zero even though queueing
demonstrably happened. Everything downstream divides by these numbers, so this is
measured before any arithmetic is written.

**Files:**
- Create: `llm/linux-turing-dual/tests/probe-queue-metrics.py` (diagnostic, kept)
- Modify: `llm/linux-turing-dual/docs/measured-ceilings.md`

**Interfaces:**
- Consumes: a running node.
- Produces: a recorded decision — either `requests_deferred` is usable, or
  `outstanding` comes from `/slots?model=<id>` `is_processing` counts instead.
  Task 2's `queue_state()` reads whichever this task selects.

- [x] **Step 1: Write the probe**

It must (a) sample `/metrics?model=<id>` and `/slots?model=<id>` every 200 ms,
(b) fire 6 requests whose prompts are long enough to occupy a slot for tens of
seconds (use the 40k needle body — a short prompt finishes before any sampler
sees it), and (c) print every distinct `(processing, deferred, slots_busy)` tuple
with timestamps.

```python
# llm/linux-turing-dual/tests/probe-queue-metrics.py
"""Diagnostic: does requests_deferred ever rise above zero?

Sampled at 200 ms with SLOW requests, because a short prompt starts and finishes
between two samples and makes an unusable metric look merely quiet.
"""
import json, subprocess, sys, threading, time

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8080"
MODEL = sys.argv[2] if len(sys.argv) > 2 else "qwen3.8-27b"
KEY = open("/tmp/k").read().strip()
N = 6

def curl(path):
    r = subprocess.run(["curl", "-sf", "-H", "Authorization: Bearer " + KEY,
                        BASE + path], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""

def metric(text, name):
    for line in text.splitlines():
        if line.startswith("llamacpp:" + name):
            return line.split()[-1]
    return None

seen, stop = [], False
def sample():
    while not stop:
        m = curl("/metrics?model=" + MODEL)
        s = curl("/slots?model=" + MODEL)
        busy = None
        try:
            busy = sum(1 for x in json.loads(s) if x.get("is_processing"))
        except Exception:
            pass
        tup = (metric(m, "requests_processing"), metric(m, "requests_deferred"), busy)
        if not seen or seen[-1][1] != tup:
            seen.append((round(time.time() - T0, 2), tup))
        time.sleep(0.2)

para = ("Routine log entry. System nominal. Subsystem checks completed without "
        "incident. Telemetry within expected bounds. No operator action required. ")
body = para * int(40000 * 5.7 / len(para))
req = {"model": MODEL, "messages": [{"role": "user", "content": body + "\n\nSummarise."}],
       "max_tokens": 32, "temperature": 0,
       "chat_template_kwargs": {"enable_thinking": False}}
with open("/tmp/probe_req.json", "w") as f:
    json.dump(req, f)

def fire(i):
    subprocess.run(["curl", "-s", "-o", "/dev/null", "--max-time", "900",
                    "-H", "Authorization: Bearer " + KEY,
                    "-H", "Content-Type: application/json",
                    BASE + "/v1/chat/completions", "-d", "@/tmp/probe_req.json"])

T0 = time.time()
sampler = threading.Thread(target=sample, daemon=True); sampler.start()
threads = [threading.Thread(target=fire, args=(i,)) for i in range(N)]
for t in threads: t.start()
for t in threads: t.join()
stop = True; time.sleep(0.5)
print("distinct (processing, deferred, slots_busy) transitions:")
for ts, tup in seen:
    print("  +%6.2fs  processing=%-4s deferred=%-4s slots_busy=%s" % (ts, *tup))
maxdef = max((int(t[1][1] or 0) for t in seen), default=0)
maxbusy = max((int(t[1][2] or 0) for t in seen), default=0)
print()
print("max deferred observed : %d" % maxdef)
print("max slots_busy        : %d" % maxbusy)
print("VERDICT: requests_deferred is %s" % ("USABLE" if maxdef > 0 else "NOT USABLE -- fall back to /slots"))
```

- [x] **Step 2: Run it against the live node**

```bash
ssh <node> 'sudo cat /etc/qwen-turing.key > /tmp/k; chmod 600 /tmp/k'
scp llm/linux-turing-dual/tests/probe-queue-metrics.py <node>:/tmp/
ssh <node> 'python3 /tmp/probe-queue-metrics.py'
```
Expected: a `VERDICT:` line. Six slow requests against two slots **must** show
four waiting at some point; if `deferred` stays 0 while `slots_busy` reaches 2,
the metric does not report waiters and the fallback is required.

- [x] **Step 3: Record the verdict**

Add a "Queue observability" section to `docs/measured-ceilings.md` with the
transition table and the decision. If `requests_deferred` is unusable, state
that `outstanding` is computed as
`slots_busy + (in-flight requests the sampler cannot see) → slots_busy only`,
and that queue depth beyond capacity is therefore **not observable from
llama.cpp** — in which case Task 2 derives `queued` from
`max(0, outstanding_seen_max - slots)` over the window and the UI labels it
"observed", not "exact".

- [x] **Step 4: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/tests/probe-queue-metrics.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "test(llm): find out what requests_deferred actually reports

A six-request burst against two slots never showed it above zero, which is either
short-lived deferral or a metric that does not mean what its name suggests. The
probe samples at 200 ms with 40k-token prompts, because a short prompt starts and
finishes between samples and makes an unusable metric look merely quiet. The
verdict is recorded before any arithmetic divides by these numbers."
```

---

### Task 2: Rolling-window queue arithmetic

**Files:**
- Create: `llm/linux-turing-dual/scripts/queue_window.py`
- Create: `llm/linux-turing-dual/tests/test_unit_queue_window.py`

**Interfaces:**
- Consumes: Task 1's verdict on which field feeds `outstanding`.
- Produces:
  `QueueWindow(window_seconds: float = 300.0)` with
  `add(ts: float, outstanding: int) -> None`,
  `samples -> int`, `completions -> int`,
  `busy_seconds -> float`,
  `service_rate() -> float | None` (completions per busy second),
  `mean_service_seconds() -> float | None`,
  `est_wait_seconds(requests_ahead: int) -> float | None`.
  Task 3 constructs one per process and calls `add()` on each poll.

- [x] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_unit_queue_window.py
"""Rolling-window queue arithmetic.

Every method must return None rather than a plausible number when it has not
observed enough to know. The whole point of this module is that "we do not know
yet" is a valid answer and a fabricated ETA is not.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "queue_window.py"


def load():
    spec = importlib.util.spec_from_file_location("queue_window", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def W(window=300.0):
    return load().QueueWindow(window_seconds=window)


# --- nothing observed ------------------------------------------------------

def test_empty_window_knows_nothing():
    w = W()
    assert w.samples == 0
    assert w.completions == 0
    assert w.service_rate() is None
    assert w.mean_service_seconds() is None
    assert w.est_wait_seconds(3) is None


def test_idle_only_still_knows_nothing():
    """An idle node has not proven it is fast."""
    w = W()
    for i in range(10):
        w.add(float(i), 0)
    assert w.completions == 0
    assert w.est_wait_seconds(2) is None


# --- completions are decreases in outstanding ------------------------------

def test_counts_a_single_completion():
    w = W()
    w.add(0.0, 1)
    w.add(1.0, 0)
    assert w.completions == 1


def test_counts_multi_request_drop_as_multiple_completions():
    w = W()
    w.add(0.0, 3)
    w.add(1.0, 1)
    assert w.completions == 2


def test_increase_is_not_a_completion():
    w = W()
    w.add(0.0, 0); w.add(1.0, 2); w.add(2.0, 5)
    assert w.completions == 0


# --- busy seconds exclude idle --------------------------------------------

def test_busy_seconds_only_counts_time_with_work_outstanding():
    w = W()
    w.add(0.0, 0)    # idle
    w.add(1.0, 1)    # becomes busy at t=1
    w.add(3.0, 0)    # idle again at t=3  -> 2 busy seconds
    w.add(9.0, 0)    # still idle
    assert w.busy_seconds == 2.0


def test_service_rate_divides_by_busy_not_wall_clock():
    """A node idle 8 of 10 seconds has not become slow."""
    w = W()
    w.add(0.0, 0); w.add(1.0, 1); w.add(3.0, 0); w.add(9.0, 0)
    # 1 completion in 2 busy seconds = 0.5/s, NOT 1/9s
    assert w.completions == 1
    assert abs(w.service_rate() - 0.5) < 1e-9
    assert abs(w.mean_service_seconds() - 2.0) < 1e-9


# --- the estimate ---------------------------------------------------------

def test_est_wait_uses_service_rate():
    w = W()
    w.add(0.0, 2); w.add(2.0, 0)   # 2 completions in 2 busy seconds -> 1/s
    assert abs(w.est_wait_seconds(4) - 4.0) < 1e-9


def test_est_wait_of_nothing_ahead_is_zero_not_none():
    w = W()
    w.add(0.0, 2); w.add(2.0, 0)
    assert w.est_wait_seconds(0) == 0.0


def test_est_wait_is_none_without_completions_even_with_samples():
    w = W()
    for i in range(50):
        w.add(float(i), 2)          # permanently busy, nothing finished
    assert w.samples == 50
    assert w.completions == 0
    assert w.est_wait_seconds(1) is None


# --- eviction ------------------------------------------------------------

def test_old_samples_leave_the_window():
    w = W(window=10.0)
    w.add(0.0, 1); w.add(1.0, 0)          # a completion at t=1
    assert w.completions == 1
    w.add(100.0, 0)                        # far outside the window
    assert w.completions == 0
    assert w.est_wait_seconds(1) is None


def test_window_keeps_recent_samples():
    w = W(window=10.0)
    w.add(0.0, 1); w.add(1.0, 0); w.add(5.0, 0)
    assert w.completions == 1


# --- robustness ----------------------------------------------------------

def test_out_of_order_timestamps_are_ignored_not_fatal():
    w = W()
    w.add(5.0, 1)
    w.add(1.0, 0)          # earlier than the last sample
    assert w.samples == 1


def test_negative_outstanding_is_clamped():
    w = W()
    w.add(0.0, -3)
    assert w.samples == 1
    assert w.completions == 0
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_queue_window.py -q`
Expected: FAIL — `queue_window.py` does not exist.

- [x] **Step 3: Write the implementation**

```python
#!/usr/bin/env python3
"""Rolling-window queue arithmetic for the dual-Turing node.

llama.cpp publishes no completed-request counter, so per-request service time
cannot be read. This module infers it from the one thing that is observable: the
number of outstanding requests going DOWN.

Every accessor returns None rather than a plausible number when it has not
observed enough to know. "We do not know yet" is a valid answer; a fabricated
ETA is not, and a confident wrong number is worse than a blank.

No I/O, no clock reads -- the caller supplies timestamps, which is what makes
this testable without sleeping.
"""
from __future__ import annotations

from collections import deque


class QueueWindow:
    def __init__(self, window_seconds: float = 300.0) -> None:
        self.window_seconds = float(window_seconds)
        # (timestamp, outstanding, completions_since_previous, busy_seconds_since_previous)
        self._samples: deque[tuple[float, int, int, float]] = deque()

    # --- ingest ---------------------------------------------------------
    def add(self, ts: float, outstanding: int) -> None:
        ts = float(ts)
        outstanding = max(0, int(outstanding))
        if self._samples and ts <= self._samples[-1][0]:
            # Out-of-order or duplicate timestamp: a negative interval would
            # corrupt busy_seconds, so drop it rather than "fix" it.
            return
        completions = 0
        busy = 0.0
        if self._samples:
            prev_ts, prev_out, _, _ = self._samples[-1]
            if outstanding < prev_out:
                completions = prev_out - outstanding
            if prev_out > 0:
                # The interval counts as busy because work was outstanding at
                # its start. Wall-clock time while idle must not dilute the rate.
                busy = ts - prev_ts
        self._samples.append((ts, outstanding, completions, busy))
        self._evict(ts)

    def _evict(self, now: float) -> None:
        cutoff = now - self.window_seconds
        while self._samples and self._samples[0][0] < cutoff:
            self._samples.popleft()

    # --- observations ---------------------------------------------------
    @property
    def samples(self) -> int:
        return len(self._samples)

    @property
    def completions(self) -> int:
        return sum(s[2] for s in self._samples)

    @property
    def busy_seconds(self) -> float:
        return sum(s[3] for s in self._samples)

    # --- derived --------------------------------------------------------
    def service_rate(self) -> float | None:
        """Completions per BUSY second, or None if nothing has completed."""
        c = self.completions
        b = self.busy_seconds
        if c <= 0 or b <= 0:
            return None
        return c / b

    def mean_service_seconds(self) -> float | None:
        r = self.service_rate()
        return None if r is None else 1.0 / r

    def est_wait_seconds(self, requests_ahead: int) -> float | None:
        """How long until `requests_ahead` requests have cleared.

        None when the rate is unknown. Zero when nothing is ahead -- that is a
        fact, not an estimate.
        """
        if requests_ahead <= 0:
            return 0.0
        r = self.service_rate()
        return None if r is None else requests_ahead / r
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_queue_window.py -q`
Expected: 14 passed

- [x] **Step 5: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/queue_window.py \
        llm/linux-turing-dual/tests/test_unit_queue_window.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): infer service rate from the only thing observable

llama.cpp has no completed-request counter, so this counts completions as
decreases in outstanding requests and divides by BUSY seconds rather than
wall-clock. A node idle for eight of ten seconds has not become slow, and
dividing by wall time would report that it had.

Every accessor returns None rather than a plausible number before it has
observed a completion, including the case of a permanently busy node where
samples accumulate and nothing finishes. A blank is a worse-looking and much
better answer than a confident fabrication.

The module takes timestamps from its caller and does no I/O, so the eviction and
rate arithmetic are tested without sleeping."
```

---

### Task 3: Public payload allow-list and sanitised `/v1/models`

The most security-relevant task. On one public port every unauthenticated route
is the attack surface, and llama.cpp's `/v1/models` currently exposes the child
argv including the API key's file path.

**Files:**
- Modify: `llm/linux-turing-dual/scripts/collect-stats.py`
- Create: `llm/linux-turing-dual/tests/test_unit_public_payload.py`

**Interfaces:**
- Consumes: `QueueWindow` from Task 2; `summarise()` from the existing collector.
- Produces:
  `PUBLIC_FIELDS: frozenset[str]`,
  `queue_state(metrics: dict, slots_total: int) -> dict`,
  `public_payload(full: dict) -> dict`,
  `sanitise_models(upstream: dict) -> dict`.
  Task 4 serves `public_payload()` on `/api/queue` and `sanitise_models()` on
  `/v1/models`.

- [x] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_unit_public_payload.py
"""What the unauthenticated surfaces may say.

llama.cpp's own /v1/models publishes each child instance's full argv without a
key -- binary path, model paths, and the API key's FILE LOCATION. That is the
standard to avoid, not to copy, so these tests assert on the key SET rather than
on individual fields: a future field added to a shared serialiser must not be
able to leak by default.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


FULL = {
    "llama_up": True,
    "model": "qwen3.8-27b",
    "prompt_tokens_total": 53958,
    "generated_tokens_total": 36,
    "tokens_per_second": 28.7,
    "kv_cache_usage_ratio": 0.42,
    "requests_processing": 1,
    "requests_deferred": 3,
    "gpu_count": 2,
    "gpu_total_mem_mib": 22528,
    "gpu_used_mem_mib": 18342,
    "gpus": [{"index": 0, "name": "NVIDIA GeForce RTX 2080 Ti", "temp_c": 48}],
    "queue": {"slots": 2, "processing": 1, "queued": 3, "fullness": 2.0,
              "est_wait_seconds": 12.0, "service_rate": 0.25,
              "mean_service_seconds": 4.0, "samples": 120, "completions": 30},
}


# --- the allow-list -------------------------------------------------------

def test_public_payload_matches_the_allow_list_exactly():
    m = load()
    p = m.public_payload(FULL)
    assert set(p.keys()) == set(m.PUBLIC_FIELDS)


def test_public_payload_drops_gpu_detail():
    """Card names and temperatures are node inventory, not load."""
    p = load().public_payload(FULL)
    assert "gpus" not in p
    assert "gpu_total_mem_mib" not in p


def test_public_payload_reports_model_loaded_as_a_boolean_not_a_name():
    p = load().public_payload(FULL)
    assert p["model_loaded"] is True
    assert "model" not in p
    assert "qwen3.8-27b" not in str(p)


def test_public_payload_has_no_path_separator_anywhere():
    """A filesystem path in a public payload is a leak regardless of its field."""
    p = load().public_payload(FULL)
    assert "/" not in str(p)


def test_public_payload_survives_a_new_field_being_added_upstream():
    m = load()
    dirty = dict(FULL, secret_path="/etc/qwen-turing.key", argv=["--api-key-file", "/x"])
    p = m.public_payload(dirty)
    assert set(p.keys()) == set(m.PUBLIC_FIELDS)
    assert "qwen-turing.key" not in str(p)


def test_public_payload_when_nothing_is_loaded():
    m = load()
    p = m.public_payload({"llama_up": False, "queue": {}})
    assert p["model_loaded"] is False
    assert p["est_wait_seconds"] is None
    assert set(p.keys()) == set(m.PUBLIC_FIELDS)


# --- queue_state ----------------------------------------------------------

def test_queue_state_reads_processing_and_deferred():
    q = load().queue_state({"llamacpp:requests_processing": 1.0,
                            "llamacpp:requests_deferred": 3.0}, slots_total=2)
    assert q["processing"] == 1
    assert q["queued"] == 3
    assert q["slots"] == 2


def test_queue_state_fullness_is_outstanding_over_slots():
    q = load().queue_state({"llamacpp:requests_processing": 1.0,
                            "llamacpp:requests_deferred": 3.0}, slots_total=2)
    assert q["fullness"] == 2.0          # (1+3)/2 -- may exceed 1.0


def test_queue_state_with_zero_slots_does_not_divide_by_zero():
    q = load().queue_state({"llamacpp:requests_processing": 0.0}, slots_total=0)
    assert q["fullness"] is None
    assert q["slots"] == 0


def test_queue_state_missing_metrics_are_zero_not_none():
    q = load().queue_state({}, slots_total=2)
    assert q["processing"] == 0 and q["queued"] == 0


# --- sanitise_models ------------------------------------------------------

UPSTREAM = {"data": [{
    "id": "qwen3.8-27b",
    "aliases": ["qwen3.8-27b-40k", "qwen3.8-27b-100k"],
    "object": "model",
    "owned_by": "llamacpp",
    "created": 1787173730,
    "status": {"value": "loaded",
               "args": ["/opt/qwen-turing/llama.cpp/current/llama-server",
                        "--api-key-file",
                        "/run/credentials/qwen-turing@router.service/apikey"]},
}]}


def test_sanitise_keeps_what_clients_need():
    d = load().sanitise_models(UPSTREAM)
    e = d["data"][0]
    assert e["id"] == "qwen3.8-27b"
    assert e["aliases"] == ["qwen3.8-27b-40k", "qwen3.8-27b-100k"]
    assert e["object"] == "model"


def test_sanitise_drops_status_and_argv():
    e = load().sanitise_models(UPSTREAM)["data"][0]
    assert "status" not in e


def test_sanitise_leaves_no_path_anywhere():
    """The whole reason this function exists."""
    d = load().sanitise_models(UPSTREAM)
    assert "/" not in str(d)
    assert "apikey" not in str(d)


def test_sanitise_of_garbage_is_an_empty_list_not_an_exception():
    m = load()
    assert m.sanitise_models({}) == {"object": "list", "data": []}
    assert m.sanitise_models({"data": "nonsense"}) == {"object": "list", "data": []}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_public_payload.py -q`
Expected: FAIL — `PUBLIC_FIELDS` does not exist.

- [x] **Step 3: Write the implementation**

Append to `collect-stats.py`:

```python
# --- the unauthenticated surface -------------------------------------------
# An explicit allow-list, not a deny-list. /api/queue and /status are reachable
# without a key on a single public port, so a field added to the full payload
# later must not be able to appear here by default.
PUBLIC_FIELDS = frozenset({
    "slots", "processing", "queued", "fullness", "est_wait_seconds",
    "service_rate", "mean_service_seconds", "samples", "completions",
    "model_loaded",
})


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


def public_payload(full: dict) -> dict:
    """Project the full summary onto the public allow-list.

    Built by iterating PUBLIC_FIELDS rather than by deleting private keys, so
    the failure mode of a forgotten field is a MISSING value, never a leak.
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
    for discovery, so it is cleaned rather than blocked -- and `status` is
    dropped wholesale rather than filtered, because that is where argv lives.
    """
    out = []
    data = (upstream or {}).get("data")
    if isinstance(data, list):
        for m in data:
            if not isinstance(m, dict):
                continue
            out.append({k: m[k] for k in _MODEL_FIELDS if k in m})
    return {"object": "list", "data": out}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_public_payload.py -q`
Expected: 14 passed

- [x] **Step 5: Run the whole node suite for regressions**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests llm/linux-qwen38/tests -q`
Expected: all pass; the existing collector tests must be unaffected.

- [x] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/collect-stats.py \
        llm/linux-turing-dual/tests/test_unit_public_payload.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): make the unauthenticated surface an allow-list

llama.cpp's /v1/models publishes each child instance's full argv without a key,
including the API key's file location. On a single public port that is the most
exposed thing on the node, so it is sanitised rather than blocked -- clients need
it for discovery -- and status is dropped wholesale rather than filtered, because
that is where argv lives.

public_payload iterates the allow-list instead of deleting private keys, so a
field someone adds upstream later goes MISSING rather than leaking. The tests
assert on the key set and on the absence of any path separator anywhere in the
payload, which is a property that survives fields nobody has thought of yet.

model_loaded is a boolean, not a name: which model is resident is configuration,
not load, and the public surfaces exist to report load."
```

---

### Task 4: Serve `/api/queue`, `/status`, `/api/queue-headers` and `/v1/models`

**Files:**
- Modify: `llm/linux-turing-dual/scripts/dashboard.py`
- Create: `llm/linux-turing-dual/web/status.html`

**Interfaces:**
- Consumes: `QueueWindow`, `queue_state`, `public_payload`, `sanitise_models`.
- Produces four routes. `/api/queue-headers` returns **204 with `X-Queue-*`
  headers and never a body**; nginx in Task 5 consumes it via `auth_request`.

- [x] **Step 1: One refresher thread owns all polling; handlers only read a cache**

**This is the load-bearing design decision of the task.** nginx runs
`auth_request` against `/api/queue-headers` before *every* inference request. If
that handler polls, then every completion waits on three HTTP round-trips and a
`nvidia-smi` fork — and Task 9 would add a `journalctl` fork on top. A six-request
burst would mean six concurrent forks to produce a number that only changes on a
one-second cadence.

Worse, `window.add()` must be called on a **fixed cadence**. Feeding it once per
inference request would add irregular samples and corrupt the completion counting
the whole ETA rests on.

So exactly one timer polls, and every handler reads a snapshot:

```python
import threading, time
from queue_window import QueueWindow

REFRESH_SECONDS = 1.0
JOURNAL_SECONDS = 30.0          # journalctl forks a process: far less often

WINDOW = QueueWindow(window_seconds=300.0)
_CACHE: dict = {"queue": {}, "llama_up": False, "gpus": []}
_CACHE_LOCK = threading.Lock()


def _poll_once() -> dict:
    """The ONLY place that talks to the backend or forks. Never called per request."""
    base = COLLECT.read_models(BASE_URL, Handler.api_key)
    model = COLLECT.pick_loaded_model(base)
    url = f"{Handler.metrics_url}?model={model}" if model else Handler.metrics_url
    metrics = COLLECT.read_metrics(url, Handler.api_key)
    slots = COLLECT.read_slot_total(BASE_URL, Handler.api_key, model)
    summary = COLLECT.summarise(metrics, COLLECT.read_gpus(), model)
    q = COLLECT.queue_state(metrics, slots)
    WINDOW.add(time.time(), q["outstanding"])     # exactly once per tick
    q.update({
        "samples": WINDOW.samples,
        "completions": WINDOW.completions,
        "service_rate": WINDOW.service_rate(),
        "mean_service_seconds": WINDOW.mean_service_seconds(),
        "est_wait_seconds": WINDOW.est_wait_seconds(q["queued"]),
    })
    summary["queue"] = q
    summary["_metrics"] = metrics                 # Task 9's cache_health reads this
    return summary


def _refresher() -> None:
    while True:
        try:
            snap = _poll_once()
            with _CACHE_LOCK:
                # Preserve whatever the slower journal timer last wrote.
                snap["config_health"] = _CACHE.get("config_health")
                _CACHE.clear()
                _CACHE.update(snap)
        except Exception:
            # A refresh failure must never kill the timer -- a dead thread would
            # freeze the cache at a stale value and look like a quiet node.
            pass
        time.sleep(REFRESH_SECONDS)


def snapshot() -> dict:
    with _CACHE_LOCK:
        return dict(_CACHE)
```

Start the thread in `main()` as `threading.Thread(target=_refresher, daemon=True).start()`.

Every route below calls `snapshot()`. **No route calls `_poll_once()`.**

- [x] **Step 2: Add the routes**

```python
        if path == "/api/queue":                 # PUBLIC
            self._send(200, json.dumps(COLLECT.public_payload(snapshot())).encode(),
                       "application/json")
            return

        if path == "/status":                    # PUBLIC
            self._send(200, STATUS_PAGE.read_bytes(), "text/html; charset=utf-8")
            return

        if path == "/v1/models":                 # PUBLIC, sanitised
            up = COLLECT.read_models(BASE_URL, Handler.api_key)
            self._send(200, json.dumps(COLLECT.sanitise_models(up)).encode(),
                       "application/json")
            return

        if path == "/api/queue-headers":          # for nginx auth_request
            # ALWAYS 204, never a body, and NEVER any I/O: this runs before every
            # inference request. It reads the cached snapshot only. nginx treats a
            # non-2xx auth_request as a rejection, so an endpoint that could be
            # slow or fail would turn a busy dashboard into a refusal of service.
            try:
                q = (snapshot().get("queue") or {})
            except Exception:
                q = {}
            def h(v):
                return "" if v is None else str(v)
            self.send_response(204)
            self.send_header("X-Queue-Slots", h(q.get("slots")))
            self.send_header("X-Queue-Processing", h(q.get("processing")))
            self.send_header("X-Queue-Depth", h(q.get("queued")))
            self.send_header("X-Queue-Fullness",
                             h(round(q["fullness"], 3) if q.get("fullness") is not None else None))
            self.send_header("X-Queue-Est-Wait-Seconds",
                             h(round(q["est_wait_seconds"]) if q.get("est_wait_seconds") is not None else None))
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
```

- [x] **Step 2b: Prove the header endpoint is cheap, not just correct**

Correctness under a *dead* backend is not the failure mode that matters here; nginx
waiting on a *slow* one is.

```bash
# 200 sequential calls must be fast, because each one gates an inference request
time (for i in $(seq 200); do curl -s -o /dev/null http://127.0.0.1:8081/api/queue-headers; done)
```
Expected: well under 5 s total (~25 ms each including curl startup). **If this
scales with backend latency, the handler is still polling** — find the call and
move it into `_poll_once()`.


- [x] **Step 3: Write the public status page**

`web/status.html`: self-contained, no external assets, polls `/api/queue` with
**no Authorization header**. Shows slots, processing, queued, a fullness bar,
estimated wait (`—` when `null`, with "not enough samples yet"), service rate and
mean service time with the sample count, and whether a model is loaded. It must
contain no model names, no paths, and no key prompt.

- [x] **Step 4: Verify the routes on the live node**

```bash
# after deploying and restarting the dashboard
curl -s -o /dev/null -w '/api/queue no key   -> %{http_code}\n' http://127.0.0.1:8081/api/queue
curl -s -o /dev/null -w '/status    no key   -> %{http_code}\n' http://127.0.0.1:8081/status
curl -s -o /dev/null -w '/api/stats no key   -> %{http_code}\n' http://127.0.0.1:8081/api/stats
curl -sf http://127.0.0.1:8081/api/queue | python3 -m json.tool
curl -sf http://127.0.0.1:8081/v1/models | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "/" not in str(d), d; print("sanitised OK:", [m["id"] for m in d["data"]])'
curl -si http://127.0.0.1:8081/api/queue-headers | grep -iE '^HTTP|^X-Queue'
```
Expected: `/api/queue` and `/status` **200**, `/api/stats` **401**, the models
list free of `/`, and `/api/queue-headers` returning `204` with five `X-Queue-*`
headers.

- [x] **Step 5: Verify the header endpoint cannot refuse service**

Run with llama.cpp deliberately stopped:
```bash
sudo systemctl stop qwen-turing@router
curl -s -o /dev/null -w 'queue-headers with backend down -> %{http_code}\n' \
  http://127.0.0.1:8081/api/queue-headers
sudo systemctl start qwen-turing@router
```
Expected: **204**. Anything else would let a dead backend turn into nginx
rejecting every inference request.

- [x] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/dashboard.py \
        llm/linux-turing-dual/scripts/collect-stats.py \
        llm/linux-turing-dual/web/status.html \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): serve queue state publicly, and headers to nginx

/api/queue and /status answer without a key and carry load only; /api/stats
still needs one. /v1/models is served sanitised so the argv leak stops at the
proxy rather than being documented as a known issue.

/api/queue-headers always returns 204 with no body, even when the backend is
down. nginx treats a non-2xx auth_request as a rejection, so an endpoint that
could fail would convert a dead dashboard into a refusal of every inference
request. Losing headers is acceptable; losing service is not, and there is a test
that stops llama.cpp to prove which one happens."
```

---

### Task 5: nginx on port 80, backends to loopback

The riskiest task: it moves the public endpoint. Nothing here is applied before
`nginx -t` passes, and the old listener stays reachable on `:8080` so a mistake
does not strand every configured client.

**Files:**
- Create: `llm/linux-turing-dual/nginx/qwen-turing.conf`
- Modify: `config/common.conf`, `config/site.conf.example`, `scripts/site.sh`,
  `scripts/serve-router.sh`, `scripts/dashboard-run.sh`,
  `scripts/install-node.sh`, `tests/test_structural.sh`

**Interfaces:**
- Consumes: Task 4's `/api/queue-headers`.
- Produces: `QT_LLAMA_PORT` (default `8090`) and `QT_PUBLIC_PORT` (default `80`)
  in `common.conf`; llama.cpp bound to `127.0.0.1:${QT_LLAMA_PORT}`; the
  dashboard bound to `127.0.0.1:${QT_DASH_PORT}`.

- [x] **Step 1: Add the new config keys**

In `common.conf`:
```bash
# llama.cpp is no longer public: nginx owns the public port and proxies to here.
QT_LLAMA_PORT="8090"
# Public port. 80 so users need no port in the URL; 8080 stays served by the same
# nginx server block so clients configured before the move keep working.
QT_PUBLIC_PORT="80"
QT_COMPAT_PORT="8080"
# A 100k prompt needs ~210 s of prefill before its first token; nginx's 60 s
# default would sever exactly the requests this node exists to serve.
QT_PROXY_READ_TIMEOUT="900s"
QT_CLIENT_MAX_BODY="512m"
```
In `site.conf.example` add `QT_DASH_ADDR` guidance for loopback, and in
`site.sh` leave `QT_REQUIRED_VARS` unchanged — these are node settings, not site
coordinates.

- [x] **Step 2: Point the backends at loopback**

`serve-router.sh`: replace `--host "${QT_NODE_ADDR}" --port "${QT_PORT}"` with
`--host 127.0.0.1 --port "${QT_LLAMA_PORT}"`, and add a comment that the public
address is nginx's business now. `dashboard-run.sh`: `--host 127.0.0.1`, and its
`--metrics-url` becomes `http://127.0.0.1:${QT_LLAMA_PORT}/metrics`.

- [x] **Step 3: Write the nginx config**

```nginx
# The ONLY public listener on this node. llama.cpp and the dashboard both bind
# loopback, so neither is reachable directly even from inside the LAN.
#
# Namespaces do not collide, which is what makes one port viable:
#   llama.cpp : /v1/*  /completion  /health  /props  /metrics  /slots
#   dashboard : /  /status  /api/*  and a SANITISED /v1/models
upstream qwen_llama     { server 127.0.0.1:8090; keepalive 8; }
upstream qwen_dashboard { server 127.0.0.1:8081; keepalive 4; }

server {
    listen 80 default_server;
    listen 8080;                    # compatibility with clients configured pre-move

    # A full-context prompt is ~230 KB at 40k tokens and more at 100k.
    client_max_body_size 512m;

    # Both directions unbuffered: completions stream token by token, and a
    # buffered request body would add minutes of latency to a 100k prompt.
    proxy_buffering off;
    proxy_request_buffering off;
    proxy_http_version 1.1;

    # Sized from measured prefill (~210 s at 100k), not from nginx's 60 s default.
    proxy_read_timeout 900s;
    proxy_send_timeout 900s;

    # --- queue headers, from a subrequest that can never reject -------------
    # /api/queue-headers always answers 204. It is internal-only so nothing
    # external can call it, and its failure must cost headers rather than
    # service -- see the dashboard route for why it never returns non-2xx.
    location = /internal-queue-headers {
        internal;
        proxy_pass http://qwen_dashboard/api/queue-headers;
        proxy_pass_request_body off;
        proxy_set_header Content-Length "";
        # Bounded FAR below proxy_read_timeout. A wedged dashboard must cost
        # headers within a second, never stall an inference request for 900s.
        proxy_connect_timeout 1s;
        proxy_send_timeout    1s;
        proxy_read_timeout    2s;
    }

    # --- sanitised model list (NOT llama.cpp's, which leaks argv) ----------
    location = /v1/models {
        proxy_pass http://qwen_dashboard/v1/models;
    }

    # llama.cpp's non-standard twin would defeat the sanitising above.
    location = /models { return 403; }

    # --- inference --------------------------------------------------------
    location ~ ^/(v1|completion|health|props|metrics|slots|tokenize|detokenize|embedding|infill|apply-template) {
        auth_request /internal-queue-headers;
        auth_request_set $q_slots      $upstream_http_x_queue_slots;
        auth_request_set $q_processing $upstream_http_x_queue_processing;
        auth_request_set $q_depth      $upstream_http_x_queue_depth;
        auth_request_set $q_full       $upstream_http_x_queue_fullness;
        auth_request_set $q_wait       $upstream_http_x_queue_est_wait_seconds;

        add_header X-Queue-Slots            $q_slots      always;
        add_header X-Queue-Processing       $q_processing always;
        add_header X-Queue-Depth            $q_depth      always;
        add_header X-Queue-Fullness         $q_full       always;
        add_header X-Queue-Est-Wait-Seconds $q_wait       always;

        # nginx turns a failed auth_request into an error for the CLIENT, so a
        # dead or slow dashboard would otherwise take inference down with it.
        # These map any subrequest failure onto a path that proxies without
        # headers. Losing headers is acceptable; losing service is not.
        error_page 500 502 503 504 = @inference_no_headers;

        proxy_pass http://qwen_llama;
        proxy_set_header Host $host;
        proxy_set_header Connection "";
    }

    location @inference_no_headers {
        internal;
        proxy_pass http://qwen_llama;
        proxy_set_header Host $host;
        proxy_set_header Connection "";
    }

    # --- dashboard --------------------------------------------------------
    location / {
        proxy_pass http://qwen_dashboard;
        proxy_set_header Host $host;
        proxy_set_header Connection "";
    }
}

# --- TLS, prepared but NOT enabled ---------------------------------------
# Uncomment once the campus CA has issued into /etc/ssl/qwen-turing/, then add
#     ufw allow from <campus-cidr> to any port 443 proto tcp
#
# Deliberately absent until then: any HTTP->HTTPS redirect and any HSTS header.
# A redirect to a port that is not listening breaks the endpoint outright, and an
# HSTS header cached before a valid certificate exists makes the node
# unreachable from browsers that saw it.
#
# server {
#     listen 443 ssl;
#     http2 on;
#     ssl_certificate     /etc/ssl/qwen-turing/fullchain.pem;
#     ssl_certificate_key /etc/ssl/qwen-turing/privkey.pem;
#     ssl_protocols TLSv1.2 TLSv1.3;
#     include /etc/nginx/snippets/qwen-turing-locations.conf;
# }
```

- [x] **Step 4: Add structural checks for the config**

Append to `tests/test_structural.sh`, as checks 8 and 9:

```bash
# --- 8: the proxy must not be able to sever long requests ----------------
n="${NODE}/nginx/qwen-turing.conf"
if [ -r "$n" ]; then
  grep -qE '^\s*proxy_buffering\s+off'         "$n" && ok "nginx: response buffering off" \
    || bad "nginx must set proxy_buffering off -- streaming completions break otherwise"
  grep -qE '^\s*proxy_request_buffering\s+off' "$n" && ok "nginx: request buffering off" \
    || bad "nginx must set proxy_request_buffering off -- a 230 KB prompt would be buffered whole"
  t="$(sed -n 's/^\s*proxy_read_timeout\s*\([0-9]\+\)s;.*/\1/p' "$n" | head -1)"
  if [ -n "$t" ] && [ "$t" -ge 600 ]; then
    ok "nginx: proxy_read_timeout ${t}s covers a 100k prefill"
  else
    bad "nginx proxy_read_timeout must be >=600s (100k prefill is ~210s + generation)"
  fi
  grep -qE '^\s*location = /models \{ return 403' "$n" \
    && ok "nginx: unsanitised /models denied" \
    || bad "nginx must deny /models -- it is the unsanitised twin of /v1/models"
fi

# --- 9: TLS must stay scaffolded, never half-enabled --------------------
if [ -r "$n" ]; then
  if grep -qE '^\s*(return\s+301\s+https|add_header\s+Strict-Transport-Security)' "$n"; then
    bad "nginx enables a redirect or HSTS before a certificate exists"
  else
    ok "nginx: no HTTPS redirect or HSTS until the cert lands"
  fi
fi
```

- [x] **Step 5: Install nginx and validate before applying**

```bash
sudo apt-get install -y --no-install-recommends nginx
sudo install -m 0644 llm/linux-turing-dual/nginx/qwen-turing.conf \
  /etc/nginx/sites-available/qwen-turing.conf
sudo ln -sf /etc/nginx/sites-available/qwen-turing.conf /etc/nginx/sites-enabled/
sudo rm -f /etc/nginx/sites-enabled/default        # it also claims :80 default_server
sudo nginx -t
```
Expected: `syntax is ok` / `test is successful`. **Do not reload on any error** —
`nginx -t` failing after `sites-enabled/default` was removed would leave port 80
unserved.

- [x] **Step 6: Restart backends onto loopback, then start nginx**

```bash
cd ~/qwen-turing-src/llm/linux-turing-dual && bash scripts/install-node.sh --skip-build
sudo systemctl restart qwen-turing@router qwen-turing-dashboard
sleep 20
sudo ss -ltnp | grep -E ':(80|8080|8081|8090)\b'
sudo systemctl enable --now nginx
sudo systemctl reload nginx
```
Expected: `127.0.0.1:8090` and `127.0.0.1:8081` for the backends, `0.0.0.0:80`
and `0.0.0.0:8080` for nginx. **A backend on `0.0.0.0` here means the loopback
change did not take and the firewall is now the only thing protecting it.**

- [x] **Step 7: Verify through the proxy, including the cases a proxy breaks**

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
# 1. a real completion
curl -sf -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Say OK"}],"max_tokens":16,"chat_template_kwargs":{"enable_thinking":false}}' \
  | python3 -c 'import json,sys; print("completion:", json.load(sys.stdin)["choices"][0]["message"]["content"])'
# 2. the queue headers
curl -si -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"hi"}],"max_tokens":8,"chat_template_kwargs":{"enable_thinking":false}}' \
  | grep -i '^x-queue'
# 3. STREAMING must arrive incrementally, not as one buffered blob
curl -N -s -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","stream":true,"messages":[{"role":"user","content":"Count to 20"}],"max_tokens":80,"chat_template_kwargs":{"enable_thinking":false}}' \
  | ts '%.s' 2>/dev/null | head -8 || \
  curl -N -s ... | head -8   # without `ts`, watch that lines appear over time
# 4. dashboard and public surfaces on the same port
curl -s -o /dev/null -w 'GET /        -> %{http_code}\n' http://127.0.0.1/
curl -s -o /dev/null -w 'GET /status  -> %{http_code}\n' http://127.0.0.1/status
curl -s -o /dev/null -w 'GET /api/queue -> %{http_code}\n' http://127.0.0.1/api/queue
curl -s -o /dev/null -w 'GET /api/stats no key -> %{http_code}\n' http://127.0.0.1/api/stats
curl -s -o /dev/null -w 'GET /models   -> %{http_code} (403 expected)\n' http://127.0.0.1/models
curl -sf http://127.0.0.1/v1/models | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "/" not in str(d); print("v1/models sanitised")'
# 5. backends NOT reachable off-loopback
curl -s -m 3 -o /dev/null -w 'llama direct -> %{http_code}\n' http://<node-addr>:8090/health || echo 'llama direct: refused (correct)'
curl -s -m 3 -o /dev/null -w 'dash  direct -> %{http_code}\n' http://<node-addr>:8081/       || echo 'dashboard direct: refused (correct)'
```
Expected: completion `OK`; five `X-Queue-*` headers; streamed lines arriving over
time rather than all at once; `/`, `/status`, `/api/queue` → 200; `/api/stats` →
401; `/models` → 403; sanitised model list; both backends refused from off-host.

- [x] **Step 7b: The failure modes a proxy introduces**

Three things that only break once nginx is in front, each verified rather than
reasoned about:

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
# 1. HEADERS ON A STREAMED RESPONSE. add_header fires when response headers are
#    sent, which for SSE is before the body flows -- so check both together, not
#    separately, because "headers work" and "streaming works" can each pass alone.
curl -N -si -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","stream":true,"messages":[{"role":"user","content":"Count to 30"}],"max_tokens":120,"chat_template_kwargs":{"enable_thinking":false}}' \
  | awk 'NR<=25{print} /^x-queue/I{h++} END{print "x-queue headers on the streamed response:", h+0}'

# 2. INFERENCE SURVIVES A DEAD DASHBOARD. This is acceptance criterion 8, and it
#    must be checked THROUGH nginx: a failed auth_request becomes a client error
#    unless the fallback location catches it.
sudo systemctl stop qwen-turing-dashboard
curl -s -o /dev/null -w '  completion with dashboard DOWN -> %{http_code}\n' \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Say OK"}],"max_tokens":8,"chat_template_kwargs":{"enable_thinking":false}}'
sudo systemctl start qwen-turing-dashboard

# 3. auth_request UNDER LOAD. Six concurrent requests each trigger a subrequest;
#    if the header endpoint is doing I/O this shows up as latency or 5xx.
for i in $(seq 6); do
  curl -s -o /dev/null -w "%{http_code} %{time_total}s\n" \
    -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
    http://127.0.0.1/v1/chat/completions \
    -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Say OK"}],"max_tokens":8,"chat_template_kwargs":{"enable_thinking":false}}' &
done; wait
```
Expected: five `x-queue` headers **on the streamed response** with lines arriving
over time; **200 with the dashboard stopped** (headers absent, service intact);
and six 200s whose `time_total` reflects model work, not added subrequest latency.

**A 500 in case 2 means the `@inference_no_headers` fallback is not wired** — fix
that before opening the firewall, because it converts a dashboard restart into an
inference outage.

- [x] **Step 8: Verify a 40k prompt survives the proxy**

Run the existing needle probe against port 80:
```bash
python3 /tmp/needle.py 40000 qwen3.8-27b     # with BASE pointed at http://127.0.0.1
```
Expected: needle retrieved. This is the case a default `proxy_read_timeout` or
request buffering would break, and it must be proven before the 100k tier is
added on top.

- [x] **Step 9: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/nginx/ llm/linux-turing-dual/config/ \
        llm/linux-turing-dual/scripts/ llm/linux-turing-dual/tests/test_structural.sh \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): put nginx on port 80 and retire the public backends

One public listener. llama.cpp moves to 127.0.0.1:8090 and the dashboard to
127.0.0.1:8081, so neither is reachable directly even from inside the LAN and the
firewall has a single port to reason about. :8080 stays served by the same server
block so clients configured before the move keep working.

Both buffering directions are off and proxy_read_timeout is 900s, sized from
measured prefill rather than from nginx's 60s default: a 100k prompt waits ~210s
for its first token, and a buffered request body would add minutes to it. Two
structural checks now fail the build if either is weakened, because both failures
look like the model hanging rather than like a proxy setting.

TLS ships commented out with no redirect and no HSTS. A redirect to a port that
is not listening breaks the endpoint, and HSTS cached before a valid certificate
makes the node unreachable -- so a third check fails the build if either appears
before the cert does."
```

---

### Task 6: The 100k context tier

**Files:**
- Modify: `config/router-presets.ini`, `config/common.conf`,
  `docs/measured-ceilings.md`

**Interfaces:**
- Consumes: a working proxy from Task 5.
- Produces served ids `qwen3.8-27b-100k`, `-50k`, `-40k` resolving to the same
  resident weights.

- [x] **Step 1: Raise the pool, and split 100k into its own preset**

Two changes to `router-presets.ini`.

First, **delete `cache-reuse = 256` from the `[*]` section.** The node reports
`cache_reuse is not supported by this context, it will be disabled` at every
load: `--cache-reuse` reuses chunks across a *changed* prefix, and this hybrid
model's Gated-DeltaNet layers hold recurrent state that cannot be partially
rewound. An option that is silently discarded must not sit in the config looking
effective.

Second, the 27B keeps two slots and gains the mid tiers, and **100k becomes a
separate preset with one slot**:

```ini
; Pool 102,400 tokens. KV is 18.0 KiB/token (16 of 64 layers are full-attention),
; so the pool costs 1.76 GiB and the resident total is ~18,702 MiB of 22,528 --
; 360 MiB more than the 81,920 pool, leaving ~3.8 GiB free.
;
; Tiers on THIS preset are a client-side contract; there is no admission control.
; 50k beside 50k is exactly one full pool. Two 100k sessions here would ask for
; 204,800 of 102,400 and both would die -- which is why 100k is a separate preset
; with parallel = 1 rather than an alias.
[qwen3.8-27b]
model = <QT_MODELS_DIR>/Qwen3.8-27B-UD-Q4_K_M.gguf
c = 102400
parallel = 2
split-mode = layer
tensor-split = 1,1
alias = qwen3.8-27b-50k,qwen3.8-27b-40k

; --- 100k, ONE slot ---------------------------------------------------------
; One slot is the feature, not a limitation. Prefix caching gives a measured 9x on
; a repeated long prompt (19.8 s -> 2.3 s, 19,425 cached tokens) but only when the
; request lands on the slot holding that prefix. With two slots and round-robin
; assignment -- slot-prompt-similarity stays 0.0, because the alternative crashes
; the model child -- a single sequential user hits the cache about half the time.
; At 100k that alternates ~3.5 minutes and ~seconds, which reads as a broken node.
;
; parallel = 1 puts every turn of a session on the same warm slot. The cost is that
; while this preset is resident the node serves ONE session, which is exactly what
; "drop a slot" meant.
[qwen3.8-27b-100k]
model = <QT_MODELS_DIR>/Qwen3.8-27B-UD-Q4_K_M.gguf
c = 102400
parallel = 1
split-mode = layer
tensor-split = 1,1
```

Both presets name the same weights file, so switching costs one reload (~7 s
measured), not a re-download.

- [x] **Step 2: Verify the preset gate accepts it**

Run: `cd /tmp/wt-turing && python3 llm/linux-qwen38/tests/check_presets.py --config llm/linux-turing-dual/config/router-presets.ini`
Expected: pass. If it rejects a tier for advertising more sessions than slots,
that is the gate working — fix the aliases, not the gate.

- [x] **Step 3: Check the arithmetic before touching the node**

```bash
python3 -c "
KV=18432; GiB=2**30; base=16902  # MiB, measured non-KV footprint
for pool in (81920,102400):
    kv=pool*KV/GiB; print(f'{pool:>7}: KV {kv:.2f} GiB, resident {base+kv*1024:.0f} MiB, free {22528-(base+kv*1024):.0f} MiB')
for seat,n in ((102400,1),(51200,2),(40960,2)):
    print(f'  tier {seat//1024:>3}k x{n} = {seat*n} of 102400 -> {\"ok\" if seat*n<=102400 else \"OVER\"}')
"
```
Expected: 102,400 → ~18,702 MiB resident, ~3,826 MiB free; all three tiers `ok`.

- [x] **Step 4: Deploy and confirm the tier is served**

```bash
cd ~/qwen-turing-src/llm/linux-turing-dual && bash scripts/install-node.sh --skip-build
sudo systemctl restart qwen-turing@router
sleep 20
curl -sf http://127.0.0.1/v1/models | python3 -c 'import json,sys; [print("  ", m["id"], m.get("aliases")) for m in json.load(sys.stdin)["data"]]'
nvidia-smi --query-gpu=index,memory.used,memory.total --format=csv,noheader
```
Expected: the aliases listed; resident VRAM near 18,700 MiB once warm.

- [x] **Step 5: Verify 100k AT LENGTH — the only claim that counts**

```bash
python3 /tmp/needle.py 100000 qwen3.8-27b-100k
```
Expected: `prompt_tokens` near 100,000 and the needle retrieved. Time it: prefill
should be ~210 s, so the whole call takes several minutes. **If it fails on a
timeout rather than on capacity, that is Task 5's `proxy_read_timeout`, not this
tier.**

- [x] **Step 5b: Verify the one-slot preset actually delivers the cache hit**

This is the entire reason it is a separate preset, so it is measured:

```bash
python3 /tmp/cachetest.py     # pointed at qwen3.8-27b-100k
```
Expected: **call 2 reports non-zero `cached_tokens`**, not only call 3. On the
two-slot preset call 2 misses because round-robin sends it to a cold slot; with
one slot there is no cold slot to land on.

**This step gates the §7.6 narrative, not just the preset.** Round-robin is the
most likely explanation for the original miss but it was inferred, not proven. If
call 2 *still* misses with `total_slots` confirmed as 1 on
`/props?model=qwen3.8-27b-100k`, then the cause was something else — chat-template
variance changing the prefix, or pool pressure evicting the slot's cache — and the
claim that a dedicated preset buys guaranteed cache hits is **wrong and must be
corrected in the spec** rather than shipped. Do not proceed to Step 6 on a failed
5b.

- [x] **Step 5c: Confirm the two-slot tier still takes two concurrent 40k sessions**

Acceptance criterion 9. A larger pool should only help, but the claim is measured
rather than assumed:

```bash
python3 /tmp/conc.py
```
Expected: both sessions HTTP 200 with the needle retrieved, and zero KV-cache
evictions in the journal.

- [x] **Step 6: Record it**

Add to `docs/measured-ceilings.md`: the pool table, the measured prompt token
count and wall time, resident VRAM at 100k depth, and set `verified_max` in
`model-artifacts.yaml` to what was actually retrieved. If 100k fails, record the
largest size that did rather than leaving the tier advertised.

- [x] **Step 7: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/config/ llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): add a 100k tier for 360 MiB

Only 16 of the 27B's 64 layers grow a KV cache, so raising the pool from 81,920
to 102,400 tokens costs 360 MiB and still leaves ~3.8 GiB free. It needs no new
preset and no reload: the tiers are aliases on the resident model, so 'dropping a
slot' means asking for -100k, which declares a solo session.

There is still no admission control, so the aliases are a client-side contract --
a 100k session beside a 40k one asks for 140k of a 102,400 pool and both die
together. check_presets.py fails the build if a tier advertises more than the pool
or the slot count can fund.

The cost is latency, not memory: at the measured 475 tok/s prefill a 100k prompt
waits ~3.5 minutes for its first token, which is why the proxy timeout is 900s and
why cache-reuse is what makes the tier usable for a coding session."
```

---

### Task 7: Dashboard queue panel and connection examples

**Files:**
- Modify: `llm/linux-turing-dual/web/index.html`

**Interfaces:**
- Consumes: `queue` in `/api/stats`, and `/v1/models` for the served ids.
- Produces: no new interfaces.

- [x] **Step 1: Add the queue panel**

Above the token panels, driven by `d.queue`:

- capacity bar: `(processing + queued) / slots`, and it **must be allowed to
  exceed 100%** visually (clamp the bar width, not the label) because an
  over-subscribed queue is exactly what an operator needs to see
- `queued` count and `processing` count against `slots`
- estimated wait: seconds, or **`—` with "not enough samples yet"** when
  `est_wait_seconds` is `null` — never render `null` as `0`
- `mean_service_seconds` and `service_rate`, each followed by
  `(n samples, m completions)` so a figure from three observations does not look
  like one from three hundred

```javascript
function renderQueue(q){
  if(!q){ return; }
  const slots = q.slots || 0;
  const out = (q.processing || 0) + (q.queued || 0);
  const pct = slots ? Math.round(out / slots * 100) : 0;
  $('qout').textContent = out + ' / ' + slots;
  // Clamp the BAR, never the number: a queue at 300% must read as 300%.
  $('qbar').style.width = Math.min(100, pct) + '%';
  $('qpct').textContent = pct + '%';
  $('qqueued').textContent = q.queued ?? '–';
  // null means "not measured yet" and must never render as 0.
  $('qwait').textContent = (q.est_wait_seconds === null || q.est_wait_seconds === undefined)
      ? '—' : Math.round(q.est_wait_seconds) + 's';
  $('qwaitnote').textContent = (q.est_wait_seconds === null || q.est_wait_seconds === undefined)
      ? 'not enough samples yet' : ('from ' + (q.completions ?? 0) + ' completions');
  $('qmean').textContent = q.mean_service_seconds
      ? q.mean_service_seconds.toFixed(1) + 's' : '—';
  $('qsamples').textContent = (q.samples ?? 0) + ' samples';
}
```

- [x] **Step 2: Add the connection-examples panel, rendered from live state**

Build it from `location.origin` and the ids returned by `/v1/models`, so the
examples cannot drift from what the node actually serves. Four tabs:

```javascript
function renderExamples(origin, ids){
  const model = ids[0] || 'qwen3.8-27b';
  return {
    'curl': `curl ${origin}/v1/chat/completions \\
  -H "Authorization: Bearer $QWEN_KEY" \\
  -H 'Content-Type: application/json' \\
  -d '{"model":"${model}",
       "messages":[{"role":"user","content":"Hello"}],
       "max_tokens":512}'`,

    'Python (OpenAI SDK)': `from openai import OpenAI
client = OpenAI(base_url="${origin}/v1", api_key="<your key>")
r = client.chat.completions.create(
    model="${model}",
    messages=[{"role": "user", "content": "Hello"}],
    max_tokens=512,          # see the note on reasoning tokens below
)
print(r.choices[0].message.content)`,

    'Node (OpenAI SDK)': `import OpenAI from "openai";
const client = new OpenAI({ baseURL: "${origin}/v1", apiKey: process.env.QWEN_KEY });
const r = await client.chat.completions.create({
  model: "${model}",
  messages: [{ role: "user", content: "Hello" }],
  max_tokens: 512,
});
console.log(r.choices[0].message.content);`,

    'OpenCode': `// ~/.config/opencode/opencode.json
{
  "provider": {
    "qwen-turing": {
      "name": "Qwen (dual Turing node)",
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "${origin}/v1", "apiKey": "<your key>" },
      "models": { ${ids.map(i => `"${i}": {}`).join(', ')} }
    }
  }
}
// Or generate it from the server so client and server cannot drift:
//   configure-opencode.py --presets /opt/qwen-turing/etc/router-presets.ini`,
  };
}
```

- [x] **Step 3: Put the two gotchas next to the examples**

They are the reasons a working node looks broken, so they belong where people
copy from, not only in the README:

1. **Reasoning tokens.** Both models emit reasoning before content, so
   `max_tokens: 16` returns **empty content**. Either allow headroom (512+) or
   send `"chat_template_kwargs": {"enable_thinking": false}`.
2. **Context tiers are a contract, not a limit.** `-100k` means *run solo*;
   `-40k`/`-50k` mean two sessions. Nothing enforces it, and a 100k session
   beside a 40k one kills both.

- [x] **Step 4: Verify in a browser and by fetch**

```bash
curl -sf http://127.0.0.1/ | grep -c 'renderQueue\|renderExamples'
curl -sf http://127.0.0.1/status | grep -ciE 'qwen3|/opt/|apikey' || echo "status page names no model or path: good"
```
Expected: both functions present in the page; the **public** status page contains
no model name and no path. Then load `http://<node-host>/` and confirm the queue
panel and examples render, and that the examples show the real origin rather than
a placeholder.

- [x] **Step 5: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/web/index.html llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): show the queue, and how to connect to it

The capacity bar clamps its width but never its label, because a queue at 300%
is exactly what an operator needs to see and a bar pinned at 100% hides it. A
null wait renders as an em dash with 'not enough samples yet' rather than as 0s,
and every derived figure carries its sample and completion counts so a number
from three observations does not look like one from three hundred.

The connection examples are built from location.origin and the live model list, so
they cannot drift from what the node serves, and they carry the two gotchas that
make a working node look broken: reasoning tokens eating a small max_tokens, and
context tiers being a client-side contract that nothing enforces."
```

---

### Task 8: Firewall, documentation, and reboot survival

**Files:**
- Modify: `ops/firewall.sh`, `README.md` (node), `llm/README.md`,
  `docs/measured-ceilings.md`

- [x] **Step 1: Open port 80, close the old backend ports**

```bash
sudo ufw allow from <campus-cidr>   to any port 80 proto tcp
sudo ufw allow from <internal-cidr> to any port 80 proto tcp
sudo ufw allow from <mgmt-cidr>     to any port 80 proto tcp
# 8080 stays open for pre-move clients; 8081 is no longer public.
sudo ufw delete allow from <internal-cidr> to any port 8081 proto tcp
sudo ufw delete allow from <mgmt-cidr>     to any port 8081 proto tcp
sudo ufw status numbered
```
No dead-man switch is needed this time: these rules only **add** an allow and
remove allows for ports nothing public listens on any more. **Verify SSH from a
second connection anyway** before moving on — the cost of being wrong is the
same as last time.

- [x] **Step 2: Confirm the exposure matches the intent**

```bash
sudo ss -ltnp | grep -E ':(80|443|8080|8081|8090)\b'
sudo ufw status | tail -12
```
Expected: nginx on `0.0.0.0:80` and `0.0.0.0:8080`; backends on `127.0.0.1` only;
no `443` listener and no `443` rule yet.

- [x] **Step 3: Document it in the node README**

Add sections for: the one-URL usage (`/`, `/status`, `/v1`), the queue panel and
what its numbers mean, **nginx as a single point of failure for inference** with
the reason it was accepted, the 100k tier including its ~3.5 minute prefill, the
reasoning-token gotcha, and the exact two-step for enabling TLS when the campus
certificate arrives.

- [x] **Step 4: Add the fourth row's detail to `llm/README.md`**

Mention that this node fronts llama.cpp with nginx and exposes queue state, so a
reader choosing between nodes knows which one has it.

- [x] **Step 5: Prove it survives a reboot — acceptance criterion 15**

```bash
sudo systemctl is-enabled nginx qwen-turing@router qwen-turing-dashboard
sudo reboot
# the host takes ~15 minutes to come back; wait, then:
systemctl is-active nginx qwen-turing@router qwen-turing-dashboard
curl -s -o /dev/null -w 'GET / -> %{http_code}\n' http://127.0.0.1/
KEY=$(sudo cat /etc/qwen-turing.key)
curl -sf -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  http://127.0.0.1/v1/chat/completions \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Say OK"}],"max_tokens":16,"chat_template_kwargs":{"enable_thinking":false}}'
sudo ufw status | head -1
nvidia-smi --query-gpu=index,memory.total,memory.free --format=csv,noheader
```
Expected: all three units active, dashboard 200, a real completion, ufw active,
both cards present. **This must be a real reboot** — `systemctl restart` does not
test unattended start, and the operator must be asked before it is taken.

- [x] **Step 6: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/ops/firewall.sh llm/linux-turing-dual/README.md \
        llm/README.md llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "docs(llm): document one URL, one public port, and a proxy that can fail

Port 80 is open to campus and the internal ranges; 8081 is closed because nothing
public listens there any more. The README states plainly that nginx is now a
single point of failure for inference -- a dead proxy is a dead endpoint -- and
why that was accepted in exchange for queue headers and for moving both backends
off every public interface.

Also records the two things that make a working node look broken: a small
max_tokens returning empty content because reasoning ran first, and context tiers
being a client-side contract that nothing enforces. Both now appear next to the
copy-paste examples rather than only in prose."
```

---

### Task 9: Config-health panel — evictions, cache rate, discarded options

The operator asked to be told when models are offloading so the configuration can
be changed to stop it. All three signals are misconfigurations, not faults.

**Files:**
- Modify: `llm/linux-turing-dual/scripts/collect-stats.py`
- Modify: `llm/linux-turing-dual/scripts/dashboard.py`
- Modify: `llm/linux-turing-dual/web/index.html`
- Create: `llm/linux-turing-dual/tests/test_unit_config_health.py`

**Interfaces:**
- Consumes: `read_metrics()` from the collector, and the unit's own journal.
- Produces:
  `parse_journal_events(text: str) -> dict` returning
  `{"evictions": [{"from": str, "to": str, "raw": str}], "unloads": [str], "disabled": [str]}`;
  `cache_health(metrics: dict) -> dict` returning
  `{"cached_tokens": int, "prompt_tokens": int, "hit_rate": float | None}`;
  `read_journal(unit: str, since: str) -> tuple[str, bool]` where the bool is
  *readable*, never conflated with *empty*. Task 7's page renders all three.

- [x] **Step 1: Write the failing test**

Create `tests/test_unit_config_health.py`. Its module docstring should say: each
signal is a fixable misconfiguration rather than a fault, and the one hard rule is
that an unreadable journal must not look like a quiet one — "no evictions" is
exactly the good news an operator would act on.

```python
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


JOURNAL = "\n".join([
    "[38805] 0.03.390.914 I srv    load_model: initializing, n_slots = 2",
    "3.00.332.657 I srv  ensure_model: evicting idle LRU name=qwen3.5-9b to make room for name=qwen3.8-27b",
    "3.00.332.658 I srv        unload: stopping model instance name=qwen3.5-9b",
    "[58527] 0.06.080.972 W srv    load_model: cache_reuse is not supported by this context, it will be disabled",
    "12.03.715.819 I srv    unload_all: stopping model instance name=qwen3.8-27b",
])


def test_parses_an_eviction_with_both_model_names():
    e = load().parse_journal_events(JOURNAL)["evictions"]
    assert len(e) == 1
    assert e[0]["from"] == "qwen3.5-9b"
    assert e[0]["to"] == "qwen3.8-27b"


def test_parses_unload_events():
    u = load().parse_journal_events(JOURNAL)["unloads"]
    assert "qwen3.5-9b" in u and "qwen3.8-27b" in u


def test_parses_silently_disabled_options():
    d = load().parse_journal_events(JOURNAL)["disabled"]
    assert any("cache_reuse" in x for x in d)


def test_ordinary_load_lines_are_not_events():
    ev = load().parse_journal_events("I srv load_model: initializing, n_slots = 2")
    assert ev == {"evictions": [], "unloads": [], "disabled": []}


def test_empty_journal_is_empty_events():
    assert load().parse_journal_events("") == {"evictions": [], "unloads": [], "disabled": []}


def test_hit_rate_from_metrics():
    c = load().cache_health({"llamacpp:prompt_tokens_total": 20000.0,
                             "llamacpp:prompt_tokens_cached_total": 19425.0})
    assert c["prompt_tokens"] == 20000
    assert c["cached_tokens"] == 19425
    assert abs(c["hit_rate"] - 0.97125) < 1e-6


def test_hit_rate_is_none_before_any_prompt():
    # Zero prompts is not a zero hit rate: one says idle, the other says thrashing.
    c = load().cache_health({})
    assert c["prompt_tokens"] == 0
    assert c["hit_rate"] is None


def test_hit_rate_zero_is_distinguishable_from_unknown():
    c = load().cache_health({"llamacpp:prompt_tokens_total": 5000.0,
                             "llamacpp:prompt_tokens_cached_total": 0.0})
    assert c["hit_rate"] == 0.0
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_config_health.py -q`
Expected: FAIL — `parse_journal_events` does not exist.

- [x] **Step 3: Write the implementation**

Append to `collect-stats.py`:

```python
import re

_EVICT_RE = re.compile(
    r"ensure_model:\s*evicting idle LRU name=(?P<frm>\S+)\s+to make room for name=(?P<to>\S+)")
_UNLOAD_RE = re.compile(r"unload(?:_all)?:\s*stopping model instance name=(?P<name>\S+)")
_DISABLED_RE = re.compile(r"(?P<opt>\w+) is not supported by this context, it will be disabled")


def parse_journal_events(text: str) -> dict:
    # Deliberately narrow patterns. A broad "unload" match would also catch the
    # ordinary shutdown of the only resident model, which is not a
    # misconfiguration and would make the eviction count meaningless.
    ev = {"evictions": [], "unloads": [], "disabled": []}
    for line in (text or "").splitlines():
        m = _EVICT_RE.search(line)
        if m:
            ev["evictions"].append({"from": m.group("frm"), "to": m.group("to"),
                                    "raw": line.strip()})
            continue
        m = _UNLOAD_RE.search(line)
        if m:
            ev["unloads"].append(m.group("name"))
            continue
        m = _DISABLED_RE.search(line)
        if m and m.group("opt") not in ev["disabled"]:
            ev["disabled"].append(m.group("opt"))
    return ev


def cache_health(metrics: dict) -> dict:
    # None, not 0.0, when nothing has been prompted: "idle" and "thrashing slots
    # and losing a 9x" must not render identically.
    total = int(metrics.get("llamacpp:prompt_tokens_total", 0) or 0)
    cached = int(metrics.get("llamacpp:prompt_tokens_cached_total", 0) or 0)
    return {"prompt_tokens": total, "cached_tokens": cached,
            "hit_rate": (cached / total) if total > 0 else None}


def read_journal(unit: str = "qwen-turing@router.service", since: str = "-2h",
                 timeout: float = 6.0) -> tuple[str, bool]:
    # The bool is READABLE. It must never be collapsed into "no events".
    import subprocess
    try:
        r = subprocess.run(["journalctl", "-u", unit, "--since", since,
                            "-o", "cat", "--no-pager"],
                           capture_output=True, text=True, timeout=timeout)
        return (r.stdout, True) if r.returncode == 0 else ("", False)
    except (OSError, subprocess.SubprocessError):
        return "", False
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_config_health.py -q`
Expected: 8 passed

- [x] **Step 5: Surface it in `/api/stats` and the page**

`read_journal` **forks a process**, so it must not run on the 1 s refresh tick and
must never be reachable from `/api/queue-headers`, which gates every inference
request. It gets its own slower timer:

```python
def _journal_refresher() -> None:
    while True:
        text, readable = COLLECT.read_journal()
        ev = COLLECT.parse_journal_events(text) if readable else None
        with _CACHE_LOCK:
            metrics = _CACHE.get("_metrics") or {}
            _CACHE["config_health"] = {
                "journal_readable": readable,
                "events": ev,
                "cache": COLLECT.cache_health(metrics),
            }
        time.sleep(JOURNAL_SECONDS)      # 30 s: a fork is not a per-second cost
```

Start it alongside `_refresher()` in `main()`. `_poll_once()` already carries
`_metrics` into the cache for `cache_health()` to read, so the cheap cache rate
still updates every second while only the event scrape is slow.

The panel renders, per signal, a count **with its window**, the last occurrence,
and a one-line remedy. When `journal_readable` is false it must show **"event feed
unavailable"** and must not render `0 evictions`.

`config_health` is **not** added to `PUBLIC_FIELDS`: eviction lines name models,
and the public surfaces carry load only.

- [x] **Step 6: Verify against a real eviction**

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
for m in qwen3.5-9b qwen3.8-27b; do
  curl -sf -o /dev/null -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
    http://127.0.0.1/v1/chat/completions \
    -d "{\"model\":\"$m\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"max_tokens\":8,\"chat_template_kwargs\":{\"enable_thinking\":false}}"
done
curl -sf -H "Authorization: Bearer $KEY" http://127.0.0.1/api/stats | python3 -c '
import json,sys
d = json.load(sys.stdin)["config_health"]
print("readable :", d["journal_readable"])
print("evictions:", d["events"]["evictions"])
print("disabled :", d["events"]["disabled"])
print("cache    :", d["cache"])'
```
Expected: at least one eviction naming both models; `disabled` **empty**, which is
the proof that removing `cache-reuse` in Task 6 worked; and a non-null hit rate.

- [x] **Step 7: Verify the journal-unavailable path**

```bash
curl -sf -H "Authorization: Bearer $KEY" http://127.0.0.1/api/stats \
  | python3 -c 'import json,sys; d=json.load(sys.stdin)["config_health"]; print("events is None when unreadable:", d["events"] is None or d["journal_readable"])'
```
Then confirm in the page that an unreadable feed reads "event feed unavailable"
rather than "0 evictions" — temporarily point `read_journal` at a nonexistent unit
to see it, and revert.

- [x] **Step 8: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/collect-stats.py \
        llm/linux-turing-dual/scripts/dashboard.py \
        llm/linux-turing-dual/web/index.html \
        llm/linux-turing-dual/tests/test_unit_config_health.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): tell the operator which configuration is costing them

Three signals, each a misconfiguration rather than a fault: model evictions from
the router's own eviction log, the prompt-cache hit rate, and options the model
silently discarded. The last is not hypothetical -- it is how cache-reuse was
caught being disabled at every load while sitting in the presets looking
effective.

An unreadable journal reports 'event feed unavailable' rather than zero
evictions, because 'no evictions' is exactly the good news an operator would act
on and the two must not look the same. For the same reason zero prompts yields a
null hit rate rather than 0%: one says idle, the other says you are thrashing
slots and losing a 9x.

The eviction patterns are deliberately narrow -- a broad unload match would also
count the ordinary shutdown of the only resident model and make the number
meaningless. config_health stays off the public allow-list, since eviction lines
name models and the public surfaces carry load only."
```

---

### Task 10: Reproducible weights — `install-node.sh` fetches from `model-artifacts.yaml`

**The repository is not currently a rebuild recipe.** `install-node.sh` accepts
`--skip-weights` but has no weights phase; the checkpoints on the node were
fetched by hand. This closes that.

**Files:**
- Modify: `llm/linux-turing-dual/scripts/install-node.sh`
- Modify: `llm/linux-turing-dual/config/model-artifacts.yaml` (add the new artifacts)
- Create: `llm/linux-turing-dual/tests/test_unit_artifacts.py`

**Interfaces:**
- Consumes: `QT_MODELS_DIR` from `site.sh`.
- Produces: `artifact_fetch_plan(yaml_text: str) -> list[dict]` in a small helper
  (`scripts/artifacts.py`) returning
  `[{"file": str, "repository": str, "revision": str, "size_bytes": int}]`,
  used by the installer and asserted by the test.

- [x] **Step 1: Write the failing test**

```python
# llm/linux-turing-dual/tests/test_unit_artifacts.py
# A truncated model is worse than a missing one: it loads, answers, and is subtly
# wrong. So the plan must carry an exact byte count for every artifact, and a
# revision rather than a branch -- the 27B repository was modified on the same day
# these weights were first fetched.
import importlib.util
import pathlib

HERE = pathlib.Path(__file__).resolve().parents[1]
SRC = HERE / "scripts" / "artifacts.py"


def load():
    spec = importlib.util.spec_from_file_location("artifacts", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def test_plan_covers_every_artifact_in_the_real_file():
    plan = load().artifact_fetch_plan((HERE / "config" / "model-artifacts.yaml").read_text())
    files = {e["file"] for e in plan}
    assert "Qwen3.8-27B-UD-Q4_K_M.gguf" in files
    assert "Qwen3.5-9B-Q4_K_M.gguf" in files
    assert any("ABLITERATED" in f for f in files), "uncensored weights missing"
    assert any("mmproj" in f for f in files), "vision projector missing"


def test_every_entry_has_a_revision_and_a_byte_count():
    for e in load().artifact_fetch_plan((HERE / "config" / "model-artifacts.yaml").read_text()):
        assert len(e["revision"]) >= 12, e
        assert e["revision"] not in ("main", "master"), e
        assert isinstance(e["size_bytes"], int) and e["size_bytes"] > 0, e


def test_no_two_entries_write_the_same_filename():
    plan = load().artifact_fetch_plan((HERE / "config" / "model-artifacts.yaml").read_text())
    files = [e["file"] for e in plan]
    assert len(files) == len(set(files)), "two artifacts would overwrite each other"


def test_empty_yaml_is_an_empty_plan_not_an_exception():
    assert load().artifact_fetch_plan("") == []
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_artifacts.py -q`
Expected: FAIL — `artifacts.py` does not exist.

- [x] **Step 3: Record the new artifacts**

Add to `model-artifacts.yaml`, each with `repository`, `revision`, `file`,
`size_bytes`, `on_disk_gib`, `served_as` and a `projector` block where relevant:

| file | repository @ revision | bytes |
|---|---|---:|
| `mmproj-F16.gguf` (27B) | `unsloth/Qwen3.8-27B-GGUF` @ `27af057ecb382ddfea5d12837360a8980560e3ed` | fetch and record |
| `Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf` | `Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF` @ `4f6732ce2123…` | fetch and record |
| `mmproj-Qwen3.8-27B-ABLITERATED-Q8_0.gguf` | same repo/revision | fetch and record |
| `mmproj-F16.gguf` (9B) | `unsloth/Qwen3.5-9B-GGUF` @ `3885219b6810b007914f3a7950a8d1b469d598a5` | fetch and record |

Get the exact sizes and the full revision hashes from the API rather than
transcribing them:
```bash
for r in unsloth/Qwen3.8-27B-GGUF unsloth/Qwen3.5-9B-GGUF Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF; do
  curl -sfL "https://huggingface.co/api/models/$r?blobs=true" | python3 -c '
import json,sys
d=json.load(sys.stdin); print(d["id"], d["sha"])
for f in d.get("siblings",[]):
    n=f.get("rfilename","")
    if "mmproj" in n or ("Q4_K_M" in n and n.endswith(".gguf")):
        print("   ", n, f.get("size"))'
done
```
**Note the two 27B `mmproj-F16.gguf` and 9B `mmproj-F16.gguf` share a filename.**
Give each a distinct local name (`mmproj-27b-F16.gguf`, `mmproj-9b-F16.gguf`) — the
duplicate-filename test exists because otherwise the second download silently
overwrites the first and the 9B would load the 27B's projector.

- [x] **Step 4: Write the helper and the installer phase**

`scripts/artifacts.py` parses the YAML with `yaml.safe_load` and flattens
artifacts plus their projectors into one list. The installer phase then:

```bash
if [ "$SKIP_WEIGHTS" -eq 0 ]; then
  say "fetch weights into ${QT_MODELS_DIR} (pinned revisions)"
  sudo install -d -m 0755 "${QT_MODELS_DIR}"
  python3 "${HERE}/artifacts.py" --plan "${NODE}/config/model-artifacts.yaml" \
  | while IFS=$'\t' read -r file repo rev size; do
      dest="${QT_MODELS_DIR}/${file}"
      have="$(stat -c%s "$dest" 2>/dev/null || echo 0)"
      if [ "$have" = "$size" ]; then
        echo "  present at expected size: ${file}"
        continue
      fi
      echo "  fetching ${file} from ${repo} @ ${rev:0:12}"
      # -C - resumes a partial file; a revision URL is immutable so resuming is safe.
      sudo curl -fL --retry 3 --retry-delay 5 -C - -o "$dest" \
        "https://huggingface.co/${repo}/resolve/${rev}/${file}"
      got="$(stat -c%s "$dest" 2>/dev/null || echo 0)"
      if [ "$got" != "$size" ]; then
        echo "  SIZE MISMATCH ${file}: got ${got}, expected ${size}" >&2
        exit 70
      fi
      echo "  ok ${file} ${got} bytes"
    done
fi
```

A truncated GGUF is worse than a missing one — it loads and answers subtly wrong —
so a mismatch is fatal rather than a warning.

- [x] **Step 5: Run test to verify it passes**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests/test_unit_artifacts.py -q`
Expected: 4 passed

- [x] **Step 6: Prove the rebuild claim — acceptance criterion 22**

A recipe nobody has run is a description. Delete one artifact and make the
installer restore it:

```bash
sudo mv /path/to/models/Qwen3.5-9B-Q4_K_M.gguf /tmp/9b.bak
cd ~/qwen-turing-src/llm/linux-turing-dual && bash scripts/install-node.sh --skip-build
ls -l "$(. scripts/site.sh; require_site; echo "$QT_MODELS_DIR")" | grep 9B
```
Expected: exactly that one file re-fetched at the right byte count, the others
reported "present at expected size", and the 9B serves a completion afterwards.
Then `rm /tmp/9b.bak`.

- [x] **Step 7: Prove a truncated file is refused**

```bash
sudo truncate -s -1024 /path/to/models/Qwen3.5-9B-Q4_K_M.gguf
bash scripts/install-node.sh --skip-build; echo "exit=$?"
```
Expected: a `SIZE MISMATCH` line and a **non-zero exit**. The installer must then
re-fetch it cleanly on the next run.

- [x] **Step 8: Commit**

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/scripts/artifacts.py \
        llm/linux-turing-dual/scripts/install-node.sh \
        llm/linux-turing-dual/config/model-artifacts.yaml \
        llm/linux-turing-dual/tests/test_unit_artifacts.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): make the repository an actual rebuild recipe

install-node.sh accepted --skip-weights and had no weights phase: the checkpoints
on this node were fetched by hand, so the repository described the node rather
than being able to reproduce it. The phase now reads model-artifacts.yaml -- the
file that already holds provenance, so a new model is added in one place -- and
fetches by pinned revision, never by branch, because the 27B repository was
modified on the same day these weights were first downloaded.

A byte-count mismatch is fatal rather than a warning. A truncated GGUF is worse
than a missing one: it loads, it answers, and it is subtly wrong.

Two projectors ship as mmproj-F16.gguf in different repositories, so they get
distinct local names and a test refuses any plan where two artifacts write the
same filename. Without that the 9B would silently load the 27B's projector.

The rebuild claim is tested by deleting one artifact and watching exactly that one
come back, because a recipe nobody has run is a description."
```

---

### Task 11: The seven-model roster and the catalog panel

**Files:**
- Modify: `llm/linux-turing-dual/config/router-presets.ini`
- Modify: `llm/linux-turing-dual/web/index.html`
- Modify: `llm/linux-turing-dual/scripts/collect-stats.py`
- Create: `llm/linux-turing-dual/tests/test_unit_catalog.py`

**Interfaces:**
- Consumes: Task 10's artifacts, Task 3's `sanitise_models`.
- Produces: `model_catalog(models: dict, presets_text: str) -> list[dict]`
  returning `[{"id", "aliases", "kind", "context", "slots", "resident"}]` where
  `kind` is one of `text`, `vision`, `uncensored`, `uncensored-vision`.

- [x] **Step 1: Add the five new presets**

Each is a separate section. **Vision is never an option on a text preset** — the
4090 node showed a projector on the shared preset costs every text-only session
~885 MiB, roughly 80k tokens of context, for a capability it never uses. Vision
presets therefore also drop to an 81,920 pool, which is how the projector is paid
for.

```ini
; --- vision: pool drops to 81,920 to pay for the 881 MiB projector -----------
; Resident 19,223 MiB of 22,528, leaving ~3,305 free.
[qwen3.8-27b-vision]
model = <QT_MODELS_DIR>/Qwen3.8-27B-UD-Q4_K_M.gguf
mmproj = <QT_MODELS_DIR>/mmproj-27b-F16.gguf
image-min-tokens = 1024
c = 81920
parallel = 2
split-mode = layer
tensor-split = 1,1
alias = qwen3.8-27b-vision-40k

; --- uncensored: Blackfrost-AI ABLITERATED, the artifact the 4090 node serves
; Abliterated builds are third-party modifications, NOT vendor artifacts: the
; revision is pinned, the size is checked, and quality is re-measured rather than
; assumed to have survived the edit. Resident 19,040 MiB, ~3,488 free.
[qwen3.8-27b-uncensored]
model = <QT_MODELS_DIR>/Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf
c = 102400
parallel = 2
split-mode = layer
tensor-split = 1,1
alias = qwen3.8-27b-uncensored-50k,qwen3.8-27b-uncensored-40k

; --- uncensored + vision: its Q8_0 projector is 604 MiB against 881 for F16,
; which is why this costs LESS than the standard vision preset.
[qwen3.8-27b-uncensored-vision]
model = <QT_MODELS_DIR>/Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf
mmproj = <QT_MODELS_DIR>/mmproj-uncensored-Q8_0.gguf
image-min-tokens = 1024
c = 81920
parallel = 2
split-mode = layer
tensor-split = 1,1

; --- 9B vision: still one card. 8,118 MiB of 11,264, ~3,146 free.
[qwen3.5-9b-vision]
model = <QT_MODELS_DIR>/Qwen3.5-9B-Q4_K_M.gguf
mmproj = <QT_MODELS_DIR>/mmproj-9b-F16.gguf
image-min-tokens = 1024
c = 81920
parallel = 2
split-mode = layer
tensor-split = 1,0
```

- [x] **Step 2: Verify the preset gate accepts all seven**

Run: `cd /tmp/wt-turing && python3 llm/linux-qwen38/tests/check_presets.py --config llm/linux-turing-dual/config/router-presets.ini`
Expected: pass. If it does not understand `mmproj`, extend it — a preset gate that
silently ignores the projector is not pricing the pool correctly.

- [x] **Step 3: Write the failing catalog test**

```python
# llm/linux-turing-dual/tests/test_unit_catalog.py
import importlib.util
import pathlib

HERE = pathlib.Path(__file__).resolve().parents[1]
SRC = HERE / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


PRESETS = HERE / "config" / "router-presets.ini"
MODELS = {"data": [
    {"id": "qwen3.8-27b", "aliases": ["qwen3.8-27b-40k"], "status": {"value": "loaded"}},
    {"id": "qwen3.8-27b-vision", "aliases": [], "status": {"value": "unloaded"}},
    {"id": "qwen3.8-27b-uncensored", "aliases": [], "status": {"value": "unloaded"}},
    {"id": "qwen3.5-9b-vision", "aliases": [], "status": {"value": "unloaded"}},
]}


def test_classifies_each_kind():
    cat = {e["id"]: e for e in load().model_catalog(MODELS, PRESETS.read_text())}
    assert cat["qwen3.8-27b"]["kind"] == "text"
    assert cat["qwen3.8-27b-vision"]["kind"] == "vision"
    assert cat["qwen3.8-27b-uncensored"]["kind"] == "uncensored"


def test_reads_context_and_slots_from_the_presets():
    cat = {e["id"]: e for e in load().model_catalog(MODELS, PRESETS.read_text())}
    assert cat["qwen3.8-27b"]["context"] == 102400
    assert cat["qwen3.8-27b"]["slots"] == 2
    # Vision pays for its projector out of the pool, and the panel must say so.
    assert cat["qwen3.8-27b-vision"]["context"] == 81920


def test_marks_exactly_one_resident():
    cat = load().model_catalog(MODELS, PRESETS.read_text())
    assert [e["id"] for e in cat if e["resident"]] == ["qwen3.8-27b"]


def test_hundred_k_preset_reports_one_slot():
    cat = {e["id"]: e for e in load().model_catalog(
        {"data": [{"id": "qwen3.8-27b-100k", "status": {"value": "unloaded"}}]},
        PRESETS.read_text())}
    assert cat["qwen3.8-27b-100k"]["slots"] == 1


def test_unknown_id_does_not_crash_the_catalog():
    cat = load().model_catalog({"data": [{"id": "not-a-preset"}]}, PRESETS.read_text())
    assert cat[0]["context"] is None and cat[0]["slots"] is None
```

- [x] **Step 4: Implement `model_catalog`**

Parse the rendered presets with `configparser`, join on the served id, and derive
`kind` from the presence of an `mmproj` key and `uncensored` in the section name.
An id with no matching section yields `None` context and slots rather than a
guess — the panel then shows `—`, which is honest.

- [x] **Step 5: Run the tests**

Run: `cd /tmp/wt-turing && python3 -m pytest llm/linux-turing-dual/tests -q`
Expected: all pass, including the earlier suites.

- [x] **Step 6: Add the catalog panel to the key-gated page**

A table: id (with aliases), kind badge, context, slots, resident marker, and an
expandable copy-paste block per id built from `location.origin`:

```javascript
function modelSnippet(origin, id, ctx){
  return `# ${id}  —  context ${ctx ? ctx.toLocaleString() : 'unknown'} tokens
curl ${origin}/v1/chat/completions \\
  -H "Authorization: Bearer $QWEN_KEY" -H 'Content-Type: application/json' \\
  -d '{"model":"${id}","messages":[{"role":"user","content":"Hello"}],"max_tokens":512}'

// ~/.config/opencode/opencode.json
{"provider":{"qwen-turing":{
  "npm":"@ai-sdk/openai-compatible",
  "options":{"baseURL":"${origin}/v1","apiKey":"<your key>"},
  "models":{"${id}":{}}}}}`;
}
```

Each row must state: **selecting a different id costs a reload** (~7 s for a 27B),
because with `--models-max 1` only one is resident. Vision rows note that an image
costs ~1,026 prompt tokens at `image-min-tokens 1024`.

The catalog stays on the **key-gated** page. It is **not** added to
`PUBLIC_FIELDS`: an inventory of which uncensored and vision models a node serves
is configuration, not load.

- [x] **Step 7: Verify every one of the seven actually answers**

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
for m in qwen3.8-27b qwen3.8-27b-100k qwen3.8-27b-vision qwen3.8-27b-uncensored \
         qwen3.8-27b-uncensored-vision qwen3.5-9b qwen3.5-9b-vision; do
  printf '%-32s ' "$m"
  curl -sf --max-time 600 -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
    http://127.0.0.1/v1/chat/completions \
    -d "{\"model\":\"$m\",\"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly: OK\"}],\"max_tokens\":32,\"chat_template_kwargs\":{\"enable_thinking\":false}}" \
    | python3 -c 'import json,sys; print(repr(json.load(sys.stdin)["choices"][0]["message"]["content"])[:40])' \
    || echo "FAILED"
done
```
Expected: `'OK'` from all seven. Each cold call includes a reload, so this loop
takes a few minutes — and every one of those reloads should appear as an eviction
in the config-health panel.

- [x] **Step 8: Verify the vision presets with a real image**

A vision preset that has never been sent an image is not verified.

```bash
python3 -c "
import base64,struct,zlib,json
# 8x8 red PNG, built here so the test needs no fixture file
def png():
    raw=b''.join(b'\x00'+b'\xff\x00\x00'*8 for _ in range(8))
    def ch(t,d):
        c=t+d; return struct.pack('>I',len(d))+c+struct.pack('>I',zlib.crc32(c))
    return (b'\x89PNG\r\n\x1a\n'+ch(b'IHDR',struct.pack('>IIBBBBB',8,8,8,2,0,0,0))
            +ch(b'IDAT',zlib.compress(raw))+ch(b'IEND',b''))
print(json.dumps({'b64': base64.b64encode(png()).decode()}))" > /tmp/img.json
B64=$(python3 -c 'import json;print(json.load(open("/tmp/img.json"))["b64"])')
for m in qwen3.8-27b-vision qwen3.8-27b-uncensored-vision qwen3.5-9b-vision; do
  printf '%-32s ' "$m"
  curl -sf --max-time 600 -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
    http://127.0.0.1/v1/chat/completions \
    -d "{\"model\":\"$m\",\"max_tokens\":48,\"chat_template_kwargs\":{\"enable_thinking\":false},
         \"messages\":[{\"role\":\"user\",\"content\":[
            {\"type\":\"text\",\"text\":\"What colour is this image? One word.\"},
            {\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,$B64\"}}]}]}" \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(repr(d["choices"][0]["message"]["content"])[:40], "prompt_tokens=", d["usage"]["prompt_tokens"])'
done
```
Expected: each names the colour, and `prompt_tokens` is ~1,026 higher than a
text-only request — the proof the projector actually ran rather than the image
being silently dropped.

- [x] **Step 9: Record and commit**

Record in `docs/measured-ceilings.md`: the roster table with measured resident
VRAM per preset, the image token cost, and the reload cost between presets.

```bash
cd /tmp/wt-turing
git add llm/linux-turing-dual/config/router-presets.ini \
        llm/linux-turing-dual/scripts/collect-stats.py \
        llm/linux-turing-dual/web/index.html \
        llm/linux-turing-dual/tests/test_unit_catalog.py \
        llm/linux-turing-dual/docs/measured-ceilings.md
git commit -m "feat(llm): serve seven models, and show what each one costs

Text, 100k, vision, uncensored, uncensored-vision, and the 9B in text and vision.
Every combination was priced against the measured non-KV footprint before being
configured, and the tightest leaves 3,146 MiB free.

Vision is a separate preset in every case, never an option on a text preset: the
4090 node established that a projector on the shared preset makes every text-only
session pay ~885 MiB -- roughly 80k tokens of context -- for a capability it never
uses. Vision presets drop to an 81,920 pool, which is how the projector is paid
for rather than pretended to be free. The uncensored projector is Q8_0 at 604 MiB,
which is why uncensored-vision costs less than standard vision.

The catalog is generated from the presets joined to the live model list, so the
context sizes and slot counts on the page cannot drift from what is served, and an
id with no matching section shows an em dash rather than a guess. It stays on the
key-gated page: an inventory of which uncensored models a node serves is
configuration, not load.

Vision presets are verified with an actual image and by prompt_tokens rising ~1,026
-- a projector that is silently dropping the image still returns a plausible
sentence about colour."
```

---

## Self-Review

**1. Spec coverage**

| spec section | task |
|---|---|
| §0 what already works | 1 (verifies the premise) |
| §1 measurement problem, null-until-observed | 2 |
| §1 open question on `requests_deferred` | **1** |
| §2 architecture, one public port | 5 |
| §3 the proxy, buffering, timeout, TLS scaffolding | 5 |
| §4 public surfaces + allow-list | 3, 4 |
| §4.1 sanitised `/v1/models`, `/models` denied | 3, 4, 5 |
| §5 dashboard queue panel | 7 |
| §6 connection examples | 7 |
| §7 exposure changes | 5, 8 |
| §7.5 100k tier + prefill latency | 6 |
| §8 acceptance 1-2 (queued>0, null ETA) | 1, 2, 4 |
| §8 acceptance 3-5 (completion, streaming, 40k, headers) | 5 |
| §8 acceptance 6-8 (public surfaces, 401, dashboard death) | 4 |
| §8 acceptance 9 (concurrency unchanged) | 6 Step 5 |
| §8 acceptance 10-11 (100k needle, check_presets) | 6 |
| §8 acceptance 12-14 (one port, backends private, no paths) | 5 Step 7 |
| §5.1 config-health panel | **9** |
| §7.6 prefix caching + one-slot 100k preset | **6** |
| §8 acceptance 15-17 (eviction shown, cache hit on call 2, journal degradation) | **9**, 6 Step 5b |
| §5.2 model catalog panel | **11** |
| §7.7 seven-model roster | **11** |
| §7.8 reproducible weights | **10** |
| §8 acceptance 18-20 (all seven answer, catalog vs public, eviction) | **11** |
| §8 acceptance 21-22 (pinned fetch, rebuild proven) | **10** |
| §8 acceptance 23 (reboot) | 8 |

**Gap found and closed:** acceptance criterion 9 (two concurrent 40k sessions
still clean after the pool change) had no task. It is now Task 6 Step 5's
companion — re-run the existing `conc.py` after raising the pool, since a larger
pool should only help but the claim is measured either way.

**Gap deliberately left open:** Task 1 may conclude `requests_deferred` is
unusable, in which case `queued` is not exactly observable and Tasks 2/4 report
an observed lower bound with the UI labelled accordingly. This is written into
Task 1 Step 3 rather than assumed away, because the alternative is arithmetic
built on a metric nobody checked.

**2. Placeholder scan:** no unresolved markers. `<node-addr>`, `<node-host>`,
`<campus-cidr>`, `<internal-cidr>` and `<mgmt-cidr>` are site values, resolved from
`/etc/qwen-turing/site.conf` and the operator's own network — deliberately not
literal, because this plan lives in a public repository alongside the spec.

**3. Type consistency:** `QueueWindow.add(ts, outstanding)` /
`service_rate()` / `mean_service_seconds()` / `est_wait_seconds(requests_ahead)`
are used identically in Tasks 2 and 4. `queue_state(metrics, slots_total)`
returns `outstanding`, which Task 4 feeds to `window.add()` and Task 7 recomputes
from `processing + queued` for display — same value, and the display path does not
depend on the extra key. `public_payload(full)` reads `full["queue"]`, which is
exactly where Task 4 puts it. `PUBLIC_FIELDS` is the single source for both the
payload and its test. `sanitise_models(upstream)` returns
`{"object": "list", "data": [...]}` in both Task 3 and Task 4's route.
`X-Queue-*` header names match between Task 4's handler and Task 5's
`auth_request_set`.
