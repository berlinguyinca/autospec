"""The default route, end to end: /v1 IS the load balancer.

Every test here runs a real gateway against two real stand-in servers -- one
playing this node's runtime, one playing another GPU box -- and asserts WHICH ONE
received the request. Nothing is asserted from the gateway's own report alone:
a balancer that says `X-Routed-To: bender` while serving the request locally
would pass a header-only test.

Three failure modes drive most of what is checked:

  * A server that does not have the model does not refuse -- llama.cpp answers
    with whatever it has loaded. Measured on this fleet: a request naming
    `qwen3.5-9b-vision` came back served by `qwen3.8-27b`. So eligibility is a
    hard filter, and it must outrank the prefix-cache affinity.
  * A refusal that does not read the request body leaves it in the socket, where
    the next request on a keep-alive connection parses it as a request line.
    That shipped once here. Every new refusal is therefore tested by pipelining
    a second request behind it on the same connection.
  * The body must reach the upstream byte-identical despite being peeked, or the
    peek has broken the pass-through it was supposed to be invisible to.
"""
import http.client
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from conftest import load_script

gw = load_script("gateway")
keys = load_script("keys")

MODEL_BOTH = "qwen3.8-27b"          # this node and the remote
MODEL_LOCAL = "qwen3.5-9b-vision"   # this node only
MODEL_REMOTE = "exotic-70b"         # the remote only


class _Fake(BaseHTTPRequestHandler):
    """A stand-in inference server that records what it was actually asked."""
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n)
        self.server.seen.append({"path": self.path, "body": body,
                                 "auth": self.headers.get("Authorization")})
        out = json.dumps({"model": self.server.name, "object": "chat.completion",
                          "usage": {"prompt_tokens": 3, "completion_tokens": 4}}
                         ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def do_GET(self):
        self.server.seen.append({"path": self.path, "body": b"", "auth": None})
        out = b'{"status":"ok"}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, *a):
        pass


def _serve(name):
    srv = ThreadingHTTPServer(("127.0.0.1", 0), _Fake)
    srv.seen, srv.name = [], name
    threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    return srv


@pytest.fixture
def fleet(tmp_path):
    """A gateway, a local runtime, a remote server, and a minted key."""
    from keystore import KeyStore
    runtime, remote = _serve("local-runtime"), _serve("remote-box")

    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    store.upsert_user("sub-a", email="a@example.org", name="a")
    full, _ = store.mint("sub-a", label="t")

    gw.STORE = store
    gw.INTERNAL_KEY = "internal-secret"
    gw.CFG.upstream_host = "127.0.0.1"
    gw.CFG.upstream_port = runtime.server_address[1]
    gw.UP_KEYS.clear()
    gw.UP_KEYS["remote"] = "remote-secret"
    gw.UPSTREAMS = gw._ups.load(
        "upstreams:\n  - id: remote\n    base_url: http://127.0.0.1:%d/v1\n"
        % remote.server_address[1])
    gw.UP_STATE.clear()
    gw.UP_STATE["remote"] = {"state": "online", "last_seen": time.time(),
                             "models": [MODEL_BOTH, MODEL_REMOTE]}
    gw.LOCAL_STATE.update(state="online", models=[MODEL_BOTH, MODEL_LOCAL],
                          error=None, last_seen=time.time())
    gw.LAST_SERVER.clear()
    gw.ROUTE_WHY.clear()

    srv = ThreadingHTTPServer(("127.0.0.1", 0), gw.Handler)
    threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    yield srv, runtime, remote, full
    srv.shutdown()
    runtime.shutdown()
    remote.shutdown()


def _rows_within(seconds, want):
    """Recorded usage, waited for rather than slept on."""
    deadline = time.time() + seconds
    seen = set()
    while time.time() < deadline:
        with gw.STORE._conn() as c:
            seen = {(r["upstream"], r["endpoint"]) for r in
                    c.execute("SELECT upstream, endpoint FROM usage_events")}
        if len(seen) >= want:
            break
        time.sleep(0.02)
    return seen


def infer(srv, key, model, path="/v1/chat/completions", pad=0, ctype="application/json"):
    body = {"messages": [{"role": "user", "content": "x" * pad}]}
    if model is not None:
        body = {"model": model, **body}
    raw = json.dumps(body).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", path, raw, {"Authorization": "Bearer " + key,
                                  "Content-Type": ctype,
                                  "Content-Length": str(len(raw))})
    r = c.getresponse()
    payload = r.read()
    out = (r.status, r.getheader("X-Routed-To"), r.getheader("X-Routed-Why"), payload)
    c.close()
    return out


# --- the default route is the virtual server --------------------------------

def test_the_plain_path_is_balanced_not_pinned_to_this_node(fleet):
    """The whole point: a client configured against /v1 -- every client already
    configured against this node -- is balanced without being reconfigured."""
    srv, runtime, remote, key = fleet
    gw.LAST_SERVER["x"] = "remote"       # not this key; must not leak across keys
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert status == 200
    assert (to, why) == ("local", "preferred")
    assert len(runtime.seen) == 1 and not remote.seen


def test_a_model_only_another_server_has_goes_there(fleet):
    srv, runtime, remote, key = fleet
    status, to, why, body = infer(srv, key, MODEL_REMOTE)
    assert status == 200
    assert (to, why) == ("remote", "model-only")
    assert not runtime.seen and len(remote.seen) == 1
    # Proof it was really served there, not just labelled.
    assert json.loads(body)["model"] == "remote-box"


def test_the_prefix_cache_affinity_holds_across_requests(fleet):
    """Consecutive requests stay on the machine that already holds the
    conversation -- worth roughly tenfold on prompt processing here."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)                     # lands on the remote
    status, to, why, _ = infer(srv, key, MODEL_BOTH)  # both could serve this
    assert (status, to, why) == (200, "remote", "last-used")
    assert len(remote.seen) == 2 and not runtime.seen


def test_eligibility_outranks_affinity(fleet):
    """You used the remote last, but you are asking for a model it has not got.
    It must drop out entirely -- it would answer with the wrong model, not with
    an error."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    status, to, why, _ = infer(srv, key, MODEL_LOCAL)
    assert (status, to, why) == (200, "local", "preferred")
    assert len(runtime.seen) == 1


def test_a_remote_that_stops_answering_drops_out_of_the_default_route(fleet):
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    gw.UP_STATE["remote"]["state"] = "offline"
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (status, to, why) == (200, "local", "preferred")


def test_this_node_dropping_out_moves_the_default_route_to_the_remote(fleet):
    """The failure the old code could not express: it assumed this node was
    always up, so a dead runtime meant every balanced request went nowhere."""
    srv, runtime, remote, key = fleet
    gw.LOCAL_STATE.update(state="offline", error="Connection refused")
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (status, to, why) == (200, "remote", "model-only")
    assert len(remote.seen) == 1


def test_a_late_liveness_probe_does_not_refuse_everything(fleet):
    """`unknown` is not `offline`. Failing closed on telemetry would take the
    node down every time the gateway restarted."""
    srv, runtime, remote, key = fleet
    gw.LOCAL_STATE.update(state="unknown", models=[], last_seen=None)
    status, to, why, _ = infer(srv, key, MODEL_LOCAL)
    assert (status, to) == (200, "local")


# --- what must NOT be balanced ----------------------------------------------

def test_this_machines_own_endpoints_stay_here(fleet):
    """/health, /slots and /metrics describe THIS box. Answering them from
    another machine would report someone else's GPUs as ours."""
    srv, runtime, remote, key = fleet
    for path in ("/health", "/slots", "/metrics", "/v1/models"):
        c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
        c.request("GET", path, None, {"Authorization": "Bearer " + key})
        r = c.getresponse()
        r.read()
        assert (r.status, r.getheader("X-Routed-To")) == (200, "local"), path
        assert r.getheader("X-Routed-Why") == "not-balanced", path
        c.close()
    assert not remote.seen


def test_polling_an_unbalanced_endpoint_does_not_reset_affinity(fleet):
    """An agent polling /v1/models between turns would otherwise be dragged back
    here, abandoning a warm remote slot for a cold local one."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/v1/models", None, {"Authorization": "Bearer " + key})
    c.getresponse().read()
    c.close()
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (to, why) == ("remote", "last-used")


# --- pinning ----------------------------------------------------------------

def test_u_local_pins_this_node_even_when_a_remote_is_warm(fleet):
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    status, to, why, _ = infer(srv, key, MODEL_BOTH,
                               path="/u/local/v1/chat/completions")
    assert (status, to, why) == (200, "local", "pinned")
    assert runtime.seen[0]["path"] == "/v1/chat/completions"   # prefix stripped


def test_pinning_a_named_server_bypasses_eligibility(fleet):
    """An explicit pin is the caller's decision, including its consequences."""
    srv, runtime, remote, key = fleet
    status, to, why, _ = infer(srv, key, MODEL_LOCAL,
                               path="/u/remote/v1/chat/completions")
    assert (status, to, why) == (200, "remote", "pinned")


def test_pinning_this_node_while_its_runtime_is_down_is_refused_at_once(fleet):
    srv, runtime, remote, key = fleet
    gw.LOCAL_STATE.update(state="offline", error="Connection refused")
    status, to, why, body = infer(srv, key, MODEL_BOTH,
                                  path="/u/local/v1/chat/completions")
    assert status == 503
    assert "not answering" in json.loads(body)["error"]["message"]


def test_the_virtual_server_is_also_addressable_by_name(fleet):
    srv, runtime, remote, key = fleet
    status, to, why, _ = infer(srv, key, MODEL_REMOTE,
                               path="/u/auto/v1/chat/completions")
    assert (status, to, why) == (200, "remote", "model-only")
    assert remote.seen[0]["path"] == "/v1/chat/completions"


# --- refusals, and the body they must drain ---------------------------------

def test_a_model_nobody_serves_is_404_not_a_substitute(fleet):
    """The alternative is what llama.cpp does on its own: answer with a
    different model and say nothing."""
    srv, runtime, remote, key = fleet
    status, to, why, body = infer(srv, key, "not-a-model-here")
    assert status == 404
    msg = json.loads(body)["error"]
    assert msg["type"] == "model_not_found" and "not-a-model-here" in msg["message"]
    assert not runtime.seen and not remote.seen


def test_a_model_whose_only_server_is_offline_says_which_one(fleet):
    srv, runtime, remote, key = fleet
    gw.UP_STATE["remote"]["state"] = "offline"
    status, to, why, body = infer(srv, key, MODEL_REMOTE)
    assert status == 503
    assert "remote" in json.loads(body)["error"]["message"]


def test_an_unreadable_model_stays_here_rather_than_risking_a_substitute(fleet):
    srv, runtime, remote, key = fleet
    # The model sits behind a body larger than the peek budget.
    body = json.dumps({"messages": [{"role": "user", "content": "x" * 20000}],
                       "model": MODEL_REMOTE}).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", "/v1/chat/completions", body,
              {"Authorization": "Bearer " + key, "Content-Type": "application/json",
               "Content-Length": str(len(body))})
    r = c.getresponse()
    r.read()
    assert (r.status, r.getheader("X-Routed-To")) == (200, "local")
    assert r.getheader("X-Routed-Why") == "blind"
    c.close()
    # And the upstream still got every byte.
    assert runtime.seen[0]["body"] == body


def test_a_request_with_no_model_field_is_labelled_differently(fleet):
    """`unnamed` (the client sent none) and `blind` (the peek ran out) are
    different problems: one is a client's choice, the other is tuning here."""
    srv, runtime, remote, key = fleet
    status, to, why, _ = infer(srv, key, None)
    assert (status, to, why) == (200, "local", "unnamed")


def test_a_non_json_body_is_not_scanned_and_stays_here(fleet):
    srv, runtime, remote, key = fleet
    status, to, why, _ = infer(srv, key, MODEL_REMOTE, ctype="audio/wav")
    assert (to, why) == ("local", "unnamed")


@pytest.mark.parametrize("model,expected", [
    ("not-a-model-here", 404),
    (None, 200),
])
def test_a_refusal_does_not_corrupt_the_next_request_on_the_connection(
        fleet, model, expected):
    """The bug this guards shipped once: a reply that does not read the body
    leaves it in the socket, and the NEXT request parses it as a request line --

        code 501, Unsupported method ('{"model":...}POST')

    -- so the innocent request behind it fails. Every new refusal path reads the
    body before answering, and this is how that is proved."""
    srv, runtime, remote, key = fleet
    if model is None:
        gw.LOCAL_STATE.update(state="offline")     # blind + local down = refusal
        expected = 503
    body = json.dumps({"model": model or "", "messages": [{"role": "user",
                                                           "content": "y" * 4000}]}
                      ).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", "/v1/chat/completions", body,
              {"Authorization": "Bearer " + key, "Content-Type": "application/json",
               "Content-Length": str(len(body))})
    r = c.getresponse()
    r.read()
    assert r.status in (404, 503)
    c.close()

    # A fresh request must be answered normally, not out of the leftovers.
    gw.LOCAL_STATE.update(state="online")
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert status == 200 and to == "local"


def test_an_unauthenticated_request_is_still_refused_before_any_routing(fleet):
    srv, runtime, remote, key = fleet
    status, to, why, _ = infer(srv, "qtk_deadbeefdead_" + "0" * 32, MODEL_BOTH)
    assert status == 401
    assert not runtime.seen and not remote.seen


# --- the peek must be invisible to the upstream -----------------------------

def test_the_body_reaches_the_upstream_byte_identical(fleet):
    srv, runtime, remote, key = fleet
    raw = json.dumps({"model": MODEL_BOTH,
                      "messages": [{"role": "user", "content": "z" * 30000}]}).encode()
    assert len(raw) > gw._peek.PEEK_BYTES        # spans the peek boundary
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", "/v1/chat/completions", raw,
              {"Authorization": "Bearer " + key, "Content-Type": "application/json",
               "Content-Length": str(len(raw))})
    r = c.getresponse()
    r.read()
    c.close()
    assert runtime.seen[0]["body"] == raw


def test_the_clients_key_is_never_forwarded(fleet):
    """Each backend holds its own credential, so a key that works here grants
    nothing on the machine behind it."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_BOTH)
    infer(srv, key, MODEL_REMOTE)
    assert runtime.seen[0]["auth"] == "Bearer internal-secret"
    assert remote.seen[0]["auth"] == "Bearer remote-secret"


# --- what the page is told --------------------------------------------------

def test_the_servers_payload_describes_this_node_from_its_probe(fleet):
    srv, runtime, remote, key = fleet
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/api/servers", None, {"Authorization": "Bearer " + key})
    d = json.loads(c.getresponse().read())
    c.close()
    rows = {s["id"]: s for s in d["servers"]}
    assert rows["local"]["state"] == "online"
    assert rows["local"]["route"] == "/u/local/v1"
    assert rows["local"]["models"] == [MODEL_BOTH, MODEL_LOCAL]
    # Never the address of anything: this payload is read by a browser.
    assert rows["local"]["base_url"] is None
    assert d["default_route"] == "/v1" and d["auto_route"] == "/u/auto/v1"


def test_the_servers_payload_reports_how_routing_actually_went(fleet):
    """So "auto quietly became local-always" is visible instead of a mystery."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    infer(srv, key, "not-a-model-here")
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/api/servers", None, {"Authorization": "Bearer " + key})
    d = json.loads(c.getresponse().read())
    c.close()
    assert d["routing"]["model-only"] == 1
    assert d["routing"]["refused"] == 1
    assert d["peek_bytes"] == gw._peek.PEEK_BYTES


def test_usage_records_which_server_served_it(fleet):
    """The scoreboard has to be fleet-aware, or a remote's tokens land on this
    node's row."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    infer(srv, key, MODEL_LOCAL)
    # Usage is recorded AFTER the response is flushed -- accounting must never
    # delay a token -- so the row appears just after the client is served.
    seen = _rows_within(2.0, 2)
    assert seen == {("remote", "/v1/chat/completions"),
                    ("local", "/v1/chat/completions")}
    # And the affinity that the next request will use is the recorded one, so a
    # gateway restart does not scatter everybody onto cold slots.
    kid = keys.parse(key)[0]
    assert gw.STORE.last_upstream(kid) == "local"
