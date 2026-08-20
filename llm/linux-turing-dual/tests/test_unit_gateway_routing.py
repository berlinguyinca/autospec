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

from nodescripts import load_script

gw = load_script("gateway")
keys = load_script("keys")
_chatmod = load_script("chat")

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
    # Nothing is measured yet, so both candidates estimate the same and the tie
    # falls to registry order, which puts this node first.
    assert (to, why) == ("local", "fastest")
    assert len(runtime.seen) == 1 and not remote.seen


def test_a_model_only_another_server_has_goes_there(fleet):
    srv, runtime, remote, key = fleet
    status, to, why, body = infer(srv, key, MODEL_REMOTE)
    assert status == 200
    assert (to, why) == ("remote", "only-server")
    assert not runtime.seen and len(remote.seen) == 1
    # Proof it was really served there, not just labelled.
    assert json.loads(body)["model"] == "remote-box"


def test_the_prefix_cache_affinity_holds_across_requests(fleet):
    """Consecutive requests stay on the machine that already holds the
    conversation -- worth roughly tenfold on prompt processing here."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)                     # lands on the remote
    status, to, why, _ = infer(srv, key, MODEL_BOTH)  # both could serve this
    assert (status, to, why) == (200, "remote", "warm")
    assert len(remote.seen) == 2 and not runtime.seen


def test_eligibility_outranks_affinity(fleet):
    """You used the remote last, but you are asking for a model it has not got.
    It must drop out entirely -- it would answer with the wrong model, not with
    an error."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    status, to, why, _ = infer(srv, key, MODEL_LOCAL)
    assert (status, to, why) == (200, "local", "only-server")
    assert len(runtime.seen) == 1


def test_a_remote_that_stops_answering_drops_out_of_the_default_route(fleet):
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    gw.UP_STATE["remote"]["state"] = "offline"
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (status, to, why) == (200, "local", "only-server")


def test_this_node_dropping_out_moves_the_default_route_to_the_remote(fleet):
    """The failure the old code could not express: it assumed this node was
    always up, so a dead runtime meant every balanced request went nowhere."""
    srv, runtime, remote, key = fleet
    gw.LOCAL_STATE.update(state="offline", error="Connection refused")
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (status, to, why) == (200, "remote", "only-server")
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
    for path in ("/health", "/slots", "/metrics"):
        c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
        c.request("GET", path, None, {"Authorization": "Bearer " + key})
        r = c.getresponse()
        r.read()
        assert (r.status, r.getheader("X-Routed-To")) == (200, "local"), path
        assert r.getheader("X-Routed-Why") == "not-balanced", path
        c.close()
    assert not remote.seen


def test_discovery_is_answered_here_rather_than_routed(fleet):
    """/v1/models is no longer proxied to anything: it is the fleet's union, which
    only this process can assemble. So it carries no routing headers -- there was
    no routing decision to report."""
    srv, runtime, remote, key = fleet
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/v1/models", None, {"Authorization": "Bearer " + key})
    r = c.getresponse()
    body = r.read()
    assert r.status == 200
    assert r.getheader("X-Routed-To") is None
    ids = [m["id"] for m in json.loads(body)["data"]]
    assert MODEL_LOCAL in ids            # from this node's own probe
    c.close()


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
    assert (to, why) == ("remote", "warm")


# --- pinning ----------------------------------------------------------------

def test_u_local_pins_this_node_even_when_a_remote_is_warm(fleet):
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)
    status, to, why, _ = infer(srv, key, MODEL_BOTH,
                               path="/u/local/v1/chat/completions")
    assert (status, to, why) == (200, "local", "pinned")
    assert runtime.seen[0]["path"] == "/v1/chat/completions"   # prefix stripped


def test_pinning_a_server_that_lacks_the_model_is_still_refused(fleet):
    """A pin names the MACHINE, not the model -- and the silent substitution does
    not care how the destination was chosen. This test previously asserted the
    opposite ("the caller's decision, including its consequences"), which left a
    documented URL returning someone else's weights with a 200."""
    srv, runtime, remote, key = fleet
    status, to, why, body = infer(srv, key, MODEL_LOCAL,
                                  path="/u/remote/v1/chat/completions")
    assert status == 404
    msg = json.loads(body)["error"]["message"]
    assert MODEL_LOCAL in msg and "remote" in msg
    assert "X-Route-Force" in msg          # the refusal says how to override it
    assert not remote.seen


def test_a_pin_can_be_forced_when_the_caller_really_means_it(fleet):
    srv, runtime, remote, key = fleet
    raw = json.dumps({"model": MODEL_LOCAL, "messages": []}).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", "/u/remote/v1/chat/completions", raw,
              {"Authorization": "Bearer " + key, "Content-Type": "application/json",
               "Content-Length": str(len(raw)), "X-Route-Force": "1"})
    r = c.getresponse()
    r.read()
    assert (r.status, r.getheader("X-Routed-To")) == (200, "remote")
    assert r.getheader("X-Routed-Why") == "forced"
    assert len(remote.seen) == 1
    c.close()


def test_pinning_a_server_whose_list_is_unknown_defers_to_it(fleet):
    """An unknown list is not evidence of absence, and this node is not the
    authority on someone else's models."""
    srv, runtime, remote, key = fleet
    gw.UP_STATE["remote"]["models"] = []
    pinned = "/u/remote/v1/chat/completions"
    status, to, why, _ = infer(srv, key, MODEL_LOCAL, path=pinned)
    assert (status, to, why) == (200, "remote", "pinned")
    assert len(remote.seen) == 1 and not runtime.seen


def test_pinning_a_non_model_endpoint_is_not_model_checked(fleet):
    srv, runtime, remote, key = fleet
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/u/remote/health", None, {"Authorization": "Bearer " + key})
    r = c.getresponse()
    r.read()
    assert (r.status, r.getheader("X-Routed-To")) == (200, "remote")
    c.close()


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
    assert (status, to, why) == (200, "remote", "only-server")
    assert remote.seen[0]["path"] == "/v1/chat/completions"


def test_affinity_survives_a_restart_without_being_faked_by_a_health_poll(fleet):
    """The in-memory guard is not enough: EVERY request is recorded, and the
    /health polls all say "local". Recovering the last server from usage has to
    ignore them, or a restart drags an agent off the warm remote it was using."""
    srv, runtime, remote, key = fleet
    infer(srv, key, MODEL_REMOTE)                       # the real routing decision
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/health", None, {"Authorization": "Bearer " + key})
    c.getresponse().read()
    c.close()
    _rows_within(2.0, 2)
    gw.LAST_SERVER.clear()                              # as a restart would
    status, to, why, _ = infer(srv, key, MODEL_BOTH)
    assert (status, to, why) == (200, "remote", "warm")


def test_a_body_with_no_content_length_is_labelled_apart(fleet):
    """Never scanned, so it must not hide inside `unnamed` and read as client
    behaviour when it is really a framing case."""
    srv, runtime, remote, key = fleet
    raw = json.dumps({"model": MODEL_REMOTE, "messages": []}).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.putrequest("POST", "/v1/chat/completions", skip_accept_encoding=True)
    c.putheader("Authorization", "Bearer " + key)
    c.putheader("Content-Type", "application/json")
    c.putheader("Transfer-Encoding", "chunked")
    c.endheaders()
    c.send(b"%x\r\n" % len(raw) + raw + b"\r\n0\r\n\r\n")
    r = c.getresponse()
    r.read()
    assert r.getheader("X-Routed-Why") == "unframed"
    assert r.getheader("X-Routed-To") == "local"
    c.close()


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
    assert d["routing"]["only-server"] == 1
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


# --- the chat panel: a session buys inference, on the same balanced path -----

def _sid(sub="sub-a"):
    import secrets
    s = secrets.token_urlsafe(8)
    gw._SESSIONS[s] = {"sub": sub, "email": f"{sub}@example.org", "name": sub,
                       "groups": [], "created": time.time()}
    return s


def chat_post(srv, sid, payload, https=True, site="same-origin", origin=None):
    raw = json.dumps(payload).encode()
    h = {"Content-Type": "application/json", "Content-Length": str(len(raw))}
    if sid:
        h["Cookie"] = f"{gw.SESSION_COOKIE}={sid}"
    if https:
        h["X-Forwarded-Proto"] = "https"
    if site:
        h["Sec-Fetch-Site"] = site
    if origin:
        h["Origin"] = origin
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=15)
    c.request("POST", "/api/chat", raw, h)
    r = c.getresponse()
    body = r.read()
    out = (r.status, r.getheader("X-Routed-To"), body)
    c.close()
    return out


def test_the_chat_panel_needs_a_session(fleet):
    srv = fleet[0]
    status, _, _ = chat_post(srv, None, {"model": MODEL_BOTH,
                                         "messages": [{"role": "user",
                                                       "content": "hi"}]})
    assert status == 401


def test_a_cross_origin_page_cannot_spend_this_node(fleet):
    """A cookie travels automatically, so without this check any page on the
    internet could make a signed-in browser buy GPU time here."""
    srv = fleet[0]
    turn = {"model": MODEL_BOTH, "messages": [{"role": "user", "content": "hi"}]}
    assert chat_post(srv, _sid(), turn, site="cross-site")[0] == 403
    assert chat_post(srv, _sid(), turn, site=None,
                     origin="https://evil.example.net")[0] == 403
    assert chat_post(srv, _sid(), turn, site=None)[0] == 403


def test_the_chat_panel_refuses_cleartext(fleet):
    srv = fleet[0]
    assert chat_post(srv, _sid(), {"model": MODEL_BOTH,
                                   "messages": [{"role": "user", "content": "hi"}]},
                     https=False)[0] == 403


def test_a_chat_turn_reaches_the_upstream_whole(fleet):
    """The rewritten body must arrive intact. The panel's request is rebuilt --
    stream, budget and thinking are added -- so it is LONGER than what the client
    sent, and a stale Content-Length would truncate the prompt upstream into
    unparseable JSON."""
    srv, runtime, _remote, _key = fleet
    prompt = "tell me about " + "x" * 3000
    status, routed, _ = chat_post(srv, _sid(), {
        "model": MODEL_LOCAL, "messages": [{"role": "user", "content": prompt}]})
    assert status == 200
    assert routed == "local"
    seen = [s for s in runtime.seen if s["path"].endswith("/v1/chat/completions")]
    assert seen, "the upstream never received the turn"
    sent = json.loads(seen[-1]["body"])
    assert sent["messages"][0]["content"] == prompt
    assert sent["stream"] is True
    assert sent["chat_template_kwargs"] == {"enable_thinking": False}
    assert sent["max_tokens"] == _chatmod.DEFAULT_MAX_TOKENS


def test_a_chat_turn_is_balanced_like_any_other_completion(fleet):
    """Not pinned to this node. A model only the remote serves must go there, or
    the panel has its own routing and will disagree with the balancer."""
    srv, _runtime, remote, _key = fleet
    status, routed, _ = chat_post(srv, _sid(), {
        "model": MODEL_REMOTE, "messages": [{"role": "user", "content": "hi"}]})
    assert status == 200
    assert routed == "remote"
    assert any(s["path"].endswith("/v1/chat/completions") for s in remote.seen)


def test_a_model_nobody_serves_never_reaches_an_upstream(fleet):
    """Refused at the panel's door rather than sent somewhere that would answer
    it with different weights and a 200."""
    srv, runtime, remote, _key = fleet
    before = len(runtime.seen) + len(remote.seen)
    status, _, body = chat_post(srv, _sid(), {
        "model": "no-such-model", "messages": [{"role": "user", "content": "hi"}]})
    assert status == 400
    assert b"does not serve" in body
    assert len(runtime.seen) + len(remote.seen) == before


def test_chat_usage_bills_the_person_without_creating_a_credential(fleet):
    """Attributed by `sub` under a sentinel key id -- so it ranks on the
    leaderboard, and no key exists that its owner cannot see or revoke."""
    srv, _runtime, _remote, _key = fleet
    chat_post(srv, _sid("sub-a"), {"model": MODEL_LOCAL,
                                   "messages": [{"role": "user", "content": "hi"}]})
    deadline = time.time() + 5
    rows = []
    while time.time() < deadline:
        with gw.STORE._conn() as c:
            rows = [dict(r) for r in c.execute(
                "SELECT key_id, sub, endpoint FROM usage_events "
                "WHERE key_id = ?", (_chatmod.USAGE_KEY_ID,))]
        if rows:
            break
        time.sleep(0.02)
    assert rows, "the chat turn was never billed"
    assert rows[0]["sub"] == "sub-a"
    assert rows[0]["endpoint"] == "/v1/chat/completions"
    assert gw.STORE.list_keys("sub-a") == [] or all(
        k.key_id != _chatmod.USAGE_KEY_ID for k in gw.STORE.list_keys("sub-a"))
