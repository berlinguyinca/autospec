"""The runtime GPU gate: refuse to serve rather than serve from a dead card.

Written against the measured failure in test_unit_health.py -- both cards took
an `Xid 154`, llama.cpp logged `failed to initialize CUDA`, six of seven models
became black holes, and the node reported `online`. Startup guards cannot catch
that: the process started correctly hours before the cards died.

The second half is about what must NOT be treated as down, and it exists because
the first version of this gate got it wrong in production. It probed nvidia-smi
from inside the gateway, whose unit sets PrivateDevices=true -- so nvidia-smi
ran, could not reach the driver, truthfully reported no cards, and the gate
503'd a node whose GPUs were entirely healthy. Hence: the verdict comes from the
dashboard (which legitimately has device access), and ANY failure to obtain one
means serve.
"""
from nodescripts import load_script

gh = load_script("gpuhealth")

# Earlier tests replace _fetch; the auth tests need the real one back.
_REAL_FETCH = gh._fetch
_REAL_CONN = gh.http.client.HTTPConnection


def stats(payload):
    gh._fetch = lambda: payload
    gh.invalidate()


# --- down means down --------------------------------------------------------

def test_a_down_verdict_refuses():
    stats({"gpu_gate": {"ok": False, "reason": "a GPU has fallen off the bus"}})
    ok, why = gh.verdict()
    assert not ok and "bus" in why


def test_a_down_verdict_without_a_reason_still_refuses_with_words():
    stats({"gpu_gate": {"ok": False}})
    ok, why = gh.verdict()
    assert not ok and why.strip() != ""


def test_healthy_serves():
    stats({"gpu_gate": {"ok": True, "reason": ""}})
    assert gh.verdict() == (True, "")


# --- what must NOT take the node down ---------------------------------------

def test_unreachable_dashboard_fails_open():
    # A monitoring outage must not become an inference outage.
    stats(None)
    ok, why = gh.verdict()
    assert ok, f"must serve when the verdict is unavailable (got {why})"


def test_dashboard_without_the_field_fails_open():
    # Rolling upgrade: gateway new, dashboard not yet restarted.
    stats({"llama_up": True, "gpus": []})
    assert gh.verdict()[0]


def test_garbage_response_fails_open():
    stats("not a dict")
    assert gh.verdict()[0]


def test_an_empty_gpus_list_alone_does_not_refuse():
    # THE REGRESSION. The gateway cannot see /dev/nvidia* (PrivateDevices=true),
    # so anything that infers "no GPUs" from the gateway's own view is wrong.
    # Only an explicit gpu_gate.ok == False may refuse.
    stats({"gpus": [], "gpu_count": 0})
    ok, why = gh.verdict()
    assert ok, f"an empty card list is not a verdict (got {why})"


# --- caching ----------------------------------------------------------------

def test_verdict_is_cached_so_the_dashboard_is_not_polled_per_request():
    calls = []
    gh._fetch = lambda: (calls.append(1), {"gpu_gate": {"ok": True}})[1]
    gh.invalidate()
    t = 1000.0
    gh.verdict(now=t)
    gh.verdict(now=t + 0.1)
    gh.verdict(now=t + 0.2)
    assert len(calls) == 1


def test_recovery_is_visible_after_ttl():
    # A latching gate would need a human to clear it, turning a five-minute
    # outage into an hour-long one.
    stats({"gpu_gate": {"ok": False, "reason": "gone"}})
    assert not gh.verdict(now=2000.0)[0]
    stats({"gpu_gate": {"ok": True}})
    assert gh.verdict(now=2000.0 + gh.TTL + 1)[0]


# --- authentication ---------------------------------------------------------
# /api/stats is authenticated. The first deployed version sent no credential,
# got 401 on every poll, and failed open -- a gate that looked installed while
# doing nothing at all. Silent no-ops are the failure mode this file guards.

def test_configure_sets_port_and_key():
    gh.configure(9999, "sekrit")
    assert gh._dash_port() == 9999 and gh._KEY == "sekrit"


def test_a_401_is_no_verdict_not_a_refusal():
    sent = {}

    class R:
        status = 401

        def read(self):
            return b""

    class C:
        def __init__(self, *a, **k):
            pass

        def request(self, m, p, headers=None):
            sent["headers"] = headers or {}

        def getresponse(self):
            return R()

        def close(self):
            pass

    gh._fetch = _REAL_FETCH
    gh.http.client.HTTPConnection = C
    try:
        gh.configure(8081, "k")
        ok, why = gh.verdict()
    finally:
        gh.http.client.HTTPConnection = _REAL_CONN
    assert ok, "a 401 must fail open, never refuse"
    assert "Bearer k" in sent["headers"].get("Authorization", "")
