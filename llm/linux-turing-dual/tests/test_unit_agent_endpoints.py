"""Attaching a server, and the two connections an agent holds.

Everything here runs against a real gateway over real sockets, with a fake agent
that speaks the actual protocol. The rules being defended:

  * a server credential authenticates an agent and NOTHING else, and a user key
    is not one -- in both directions;
  * a server's liveness is its control connection, including the half-open case
    where the box lost power rather than closing;
  * one name, one live connection: two boxes fighting over an id would flap the
    fleet, so the second is refused with a code that says why;
  * attaching is self-service, promotion into the default pool is not.
"""
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from nodescripts import load_script
from fakeagent import FakeAgent

gw = load_script("gateway")
keys = load_script("keys")
ws = load_script("wsframe")


class Target(BaseHTTPRequestHandler):
    """Stands in for whatever OpenAI-compatible server the agent fronts."""
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        body = json.dumps({"object": "list", "data": [
            {"id": "llama-3.3-70b", "object": "model"},
            {"id": "mixtral-8x7b", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        self.server.seen.append(self.rfile.read(n))
        body = json.dumps({"model": "llama-3.3-70b",
                           "usage": {"prompt_tokens": 5, "completion_tokens": 6}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


@pytest.fixture
def node(tmp_path):
    from keystore import KeyStore
    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    store.upsert_user("sub-a", email="a@example.org", name="A")
    store.upsert_user("sub-admin", email="adm@example.org", name="Adm")

    gw.STORE = store
    gw.CFG.user_group = "*"
    gw.CFG.admin_group = "llm-admins"
    gw.CFG.public_fqdn = "node.invalid"
    gw.UPSTREAMS = []
    gw.UP_STATE.clear()
    gw.UP_KEYS.clear()
    gw.AGENT_STATE.clear()
    gw.CONTROL.clear()
    gw.INFLIGHT.clear()
    gw.ROUTE_WHY.clear()
    gw.LAST_SERVER.clear()
    gw.THROUGHPUT = {}
    gw.POOL = gw._tunnel.PipePool()
    gw.LOCAL_STATE.update(state="online", models=["local-model"], error=None,
                          last_seen=time.time())

    target = ThreadingHTTPServer(("127.0.0.1", 0), Target)
    target.seen = []
    threading.Thread(target=target.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    srv = ThreadingHTTPServer(("127.0.0.1", 0), gw.Handler)
    threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    yield srv, store, target
    srv.shutdown()
    target.shutdown()


def session(sub, groups=()):
    import secrets
    sid = secrets.token_urlsafe(8)
    gw._SESSIONS[sid] = {"sub": sub, "email": f"{sub}@example.org", "name": sub,
                         "groups": list(groups), "created": time.time()}
    return sid


def call(srv, method, path, sid=None, body=None, key=None, https=True):
    import http.client
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    h = {"Content-Type": "application/json"}
    if https:
        h["X-Forwarded-Proto"] = "https"
    if sid:
        h["Cookie"] = f"{gw.SESSION_COOKIE}={sid}"
    if key:
        h["Authorization"] = "Bearer " + key
    c.request(method, path, json.dumps(body) if body is not None else None, h)
    r = c.getresponse()
    raw = r.read()
    c.close()
    try:
        return r.status, json.loads(raw or b"{}")
    except ValueError:
        return r.status, {"raw": raw[:200]}


def attach(srv, sid, server_id="box", **kw):
    code, d = call(srv, "POST", "/api/servers/enrol", sid,
                   dict(server_id=server_id, **kw))
    assert code == 201, d
    code, out = call(srv, "POST", "/api/agent/enrol", body={"token": d["token"]})
    assert code == 201, out
    return out["credential"], d


def wait_for(predicate, seconds=3.0):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.02)
    return False


# --- enrolment --------------------------------------------------------------

def test_any_signed_in_member_may_attach_a_server(node):
    srv, store, _ = node
    cred, issued = attach(srv, session("sub-a"))
    assert cred.startswith("qts_")
    assert issued["kind"] == "tunnel"
    assert "qwen-turing-agent enrol" in issued["command"]
    assert "node.invalid" in issued["command"]


def test_the_token_is_shown_once_and_works_once(node):
    srv, _, _ = node
    code, d = call(srv, "POST", "/api/servers/enrol", session("sub-a"),
                   {"server_id": "box"})
    assert (code, d["shown_once"]) == (201, True)
    assert call(srv, "POST", "/api/agent/enrol", body={"token": d["token"]})[0] == 201
    assert call(srv, "POST", "/api/agent/enrol", body={"token": d["token"]})[0] == 401


def test_reenrolment_immediately_discards_cached_routing_evidence(monkeypatch, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    store.record_usage({
        "ts": time.time() - 1, "key_id": "k", "sub": "sub-a", "model": "m1",
        "upstream": "box", "prompt_tokens": 1000, "completion_tokens": 10,
        "prompt_ms": 1000, "predicted_ms": 100, "status_code": 200,
    })
    gw._refresh_throughput()
    assert ("box", "m1") in gw.THROUGHPUT

    assert call(srv, "DELETE", "/api/servers/box", session("sub-a"))[0] == 200
    attach(srv, session("sub-a"), "box")
    assert ("box", "m1") not in gw.THROUGHPUT

    _said("box", {
        "type": "capabilities", "hardware_fingerprint": "v1:baseline", "cards": []
    }, monkeypatch)
    assert ("box", "m1") not in gw.THROUGHPUT


def test_reenrolment_survives_a_failed_full_cache_refresh(
        monkeypatch, capsys, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    assert call(srv, "DELETE", "/api/servers/box", session("sub-a"))[0] == 200
    code, issued = call(srv, "POST", "/api/servers/enrol", session("sub-a"),
                        {"server_id": "box"})
    assert code == 201
    gw.THROUGHPUT = {
        ("box", "m1"): {"samples": 7},
        ("box", None): {"samples": 7},
        ("other", "m1"): {"samples": 3},
    }

    def failed_scan():
        raise RuntimeError("scan failed\nSECOND LINE MUST NOT BE LOGGED" + "x" * 500)

    monkeypatch.setattr(store, "throughput", failed_scan)
    code, enrolled = call(srv, "POST", "/api/agent/enrol",
                          body={"token": issued["token"]})

    assert code == 201
    assert all(key[0] != "box" for key in gw.THROUGHPUT)
    assert ("other", "m1") in gw.THROUGHPUT
    authenticated = store.authenticate_server(enrolled["credential"])
    assert authenticated is not None and authenticated.server_id == "box"
    assert call(srv, "POST", "/api/agent/enrol",
                body={"token": issued["token"]})[0] == 401
    error = capsys.readouterr().err
    line = next(line for line in error.splitlines()
                if line.startswith("throughput refresh after enrolment:"))
    assert "scan failed" in line and len(line) <= 160
    assert "SECOND LINE MUST NOT BE LOGGED" not in error


def test_attaching_over_cleartext_is_refused(node):
    srv, _, _ = node
    code, d = call(srv, "POST", "/api/servers/enrol", session("sub-a"),
                   {"server_id": "box"}, https=False)
    assert code == 400 and d["error"]["type"] == "insecure_transport"


def test_attaching_without_a_session_is_refused(node):
    srv, _, _ = node
    assert call(srv, "POST", "/api/servers/enrol", None, {"server_id": "box"})[0] == 401


def test_a_bad_server_name_is_explained_rather_than_accepted(node):
    srv, _, _ = node
    code, d = call(srv, "POST", "/api/servers/enrol", session("sub-a"),
                   {"server_id": "Not A Name"})
    assert code == 400 and "lowercase" in d["error"]["message"]


def test_a_static_server_is_attached_through_the_same_flow(node):
    """The other half of the problem this design set out to remove: registering a
    directly reachable box should not mean editing a file on the node either."""
    srv, store, _ = node
    cred, issued = attach(srv, session("sub-a"), "static1", kind="static",
                          base_url="http://box.invalid:8000/v1")
    assert issued["kind"] == "static"
    assert store.server("static1").base_url == "http://box.invalid:8000/v1"


# --- the control connection -------------------------------------------------

def test_a_server_credential_opens_a_control_connection(node):
    srv, _, _ = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    assert agent.control("box") == 101
    assert wait_for(lambda: gw.AGENT_STATE.get("box", {}).get("state") == "online")
    assert wait_for(lambda: gw.AGENT_STATE["box"].get("agent_version") == "test")
    agent.stop()


def test_a_user_key_does_not_open_a_control_connection(node):
    """The single most important refusal here: a key that may SPEND capacity must
    not be able to offer it."""
    srv, store, _ = node
    user, _ = store.mint("sub-a", label="k")
    attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], user)
    assert agent.control("box") == 401
    assert gw.AGENT_STATE.get("box") is None


def test_no_credential_at_all_is_refused(node):
    srv, _, _ = node
    attach(srv, session("sub-a"))
    assert FakeAgent(srv.server_address[1], "").control("box") == 401


def test_a_control_connection_over_cleartext_is_refused(node):
    srv, _, _ = node
    cred, _ = attach(srv, session("sub-a"))
    import socket
    s = socket.create_connection(("127.0.0.1", srv.server_address[1]), timeout=5)
    s.sendall(b"GET /api/agent/control HTTP/1.1\r\nHost: x\r\n"
              b"Upgrade: websocket\r\nConnection: Upgrade\r\n"
              b"Sec-WebSocket-Version: 13\r\n"
              b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
              + f"Authorization: Bearer {cred}\r\n\r\n".encode())
    status = s.makefile("rb").readline()
    s.close()
    assert b"400" in status


def test_a_second_connection_for_one_name_is_refused_with_a_reason(node):
    """Two boxes fighting over an id would flap the fleet. The loser is told."""
    srv, _, _ = node
    cred, _ = attach(srv, session("sub-a"))
    first = FakeAgent(srv.server_address[1], cred)
    assert first.control("box") == 101
    assert wait_for(lambda: "box" in gw.CONTROL)
    second = FakeAgent(srv.server_address[1], cred)
    assert second.control("box") == 101          # upgraded, then closed
    assert wait_for(lambda: second.close_code == ws.CLOSE_ALREADY_CONNECTED)
    assert gw.AGENT_STATE["box"]["state"] == "online"    # the first still holds it
    first.stop()
    second.stop()


def test_losing_the_connection_marks_the_server_offline_and_drops_its_pipes(node):
    srv, _, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") == 1)
    agent.stop_control()
    assert wait_for(lambda: gw.AGENT_STATE["box"]["state"] == "offline")
    assert gw.POOL.idle("box") == 0
    agent.stop()


def test_the_reported_state_follows_the_connection_without_waiting_for_a_poll(node):
    """Asserted through /api/servers, not through the internal dict.

    An earlier test checked AGENT_STATE and passed while the PANEL still said
    online for up to a poll interval -- observed live on the real fleet. What a
    reader sees is the thing that has to be right.
    """
    srv, _, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") == 1)
    gw._probe_tunnel("box")            # as the timer would, once

    def reported():
        rows = call(srv, "GET", "/api/servers", session("sub-a"))[1]["servers"]
        return {r["id"]: r for r in rows}["box"]

    assert reported()["state"] == "online"
    agent.stop_control()
    # No poll happens in between: the connection going away is the signal.
    assert wait_for(lambda: reported()["state"] == "offline", 3.0)
    # And the model list survives, because that is a question only the server
    # can answer and losing it would empty the panel on every blip.
    assert reported()["models"] == ["llama-3.3-70b", "mixtral-8x7b"]
    agent.stop()


def test_a_box_that_stops_answering_is_reaped_by_the_heartbeat(node, monkeypatch):
    """The half-open case: the socket is still open because the machine lost power
    rather than closing. Nothing arrives and nothing errors, so only the missing
    pong distinguishes it from an idle but healthy agent."""
    srv, _, _ = node
    monkeypatch.setattr(gw, "HEARTBEAT_SECONDS", 1)
    monkeypatch.setattr(gw, "HEARTBEAT_GRACE", 1)
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    assert wait_for(lambda: gw.AGENT_STATE.get("box", {}).get("state") == "online")
    agent.go_silent()
    assert wait_for(lambda: gw.AGENT_STATE["box"]["state"] == "offline", 8.0)
    agent.stop()


# --- pipes ------------------------------------------------------------------

def test_a_pipe_is_offered_and_reported(node):
    srv, _, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    assert agent.add_pipe(("127.0.0.1", target.server_address[1])) == 101
    assert wait_for(lambda: gw.POOL.idle("box") == 1)
    rows = {s["id"]: s for s in call(srv, "GET", "/api/servers",
                                    key=None, sid=session("sub-a"))[1]["servers"]}
    assert rows["box"]["kind"] == "tunnel"
    assert rows["box"]["idle_pipes"] == 1
    assert rows["box"]["base_url"] is None       # it has no address
    agent.stop()


def test_a_pipe_needs_a_server_credential_too(node):
    srv, store, target = node
    attach(srv, session("sub-a"))
    user, _ = store.mint("sub-a", label="k")
    agent = FakeAgent(srv.server_address[1], user)
    assert agent.add_pipe(("127.0.0.1", target.server_address[1])) == 401
    assert gw.POOL.idle("box") == 0


def test_the_model_list_comes_from_the_server_itself(node):
    """Provider-agnostic: the node never derives ids from a naming convention,
    so a llama or mixtral box works exactly as a qwen one does."""
    srv, _, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") == 1)
    gw._probe_tunnel("box")
    assert gw.UP_STATE["box"]["models"] == ["llama-3.3-70b", "mixtral-8x7b"]
    agent.stop()


def test_a_dialled_server_that_wants_a_credential_says_so(node):
    """It is answering, so it is not offline -- it is refusing US. Without this it
    sat `online` with an empty model list, permanently ineligible, and the panel
    offered no reason at all."""
    srv, store, _ = node

    class Refuser(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_GET(self):
            self.send_response(401)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *a):
            pass

    ref = ThreadingHTTPServer(("127.0.0.1", 0), Refuser)
    threading.Thread(target=ref.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    try:
        attach(srv, session("sub-a"), "keyed", kind="static",
               base_url=f"http://127.0.0.1:{ref.server_address[1]}/v1")
        gw._poll_upstreams()
        st = gw.UP_STATE["keyed"]
        assert st["state"] == "online" and st["models"] == []
        assert "credential" in st["error"] and "401" in st["error"]
        # And it says what to do about it, rather than only what happened.
        assert "key_file" in st["error"] or "loopback" in st["error"]
    finally:
        ref.shutdown()


# --- promotion and detaching ------------------------------------------------

def test_promotion_needs_an_admin(node):
    srv, store, _ = node
    attach(srv, session("sub-a"))
    assert call(srv, "POST", "/api/servers/box/pool", session("sub-a"),
                {"pool_member": True})[0] == 403
    assert store.server("box").pool_member is False
    assert call(srv, "POST", "/api/servers/box/pool", session("sub-admin", ["llm-admins"]),
                {"pool_member": True})[0] == 200
    assert store.server("box").pool_member is True


def test_the_tier_needs_an_admin_and_is_bounded(node):
    srv, store, _ = node
    attach(srv, session("sub-a"))
    adm = session("sub-admin", ["llm-admins"])
    assert call(srv, "POST", "/api/servers/box/priority", session("sub-a"),
                {"priority": 3})[0] == 403
    assert call(srv, "POST", "/api/servers/box/priority", adm, {"priority": 3})[0] == 200
    assert store.server("box").priority == 3
    code, d = call(srv, "POST", "/api/servers/box/priority", adm, {"priority": 99})
    assert code == 400 and store.server("box").priority == 3


def test_the_owner_may_detach_and_a_stranger_may_not(node):
    srv, store, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") == 1)
    assert call(srv, "DELETE", "/api/servers/box", session("sub-b"))[0] == 404
    assert call(srv, "DELETE", "/api/servers/box", session("sub-a"))[0] == 200
    # Its credential is dead and its pipes are no longer capacity.
    assert gw.POOL.idle("box") == 0
    assert store.authenticate_server(cred) is None
    agent.stop()


# --- discovery --------------------------------------------------------------

def test_the_model_list_is_the_fleet_s_union_not_this_node_s(node):
    """The gap this closes: a client pointed at /v1 could be routed to a remote
    for a model its own model list had never offered it, and a model that WAS
    reachable looked unavailable."""
    srv, store, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") >= 2)
    gw._probe_tunnel("box")
    store.set_pool_member("box", True)

    code, d = call(srv, "GET", "/v1/models")
    assert code == 200
    ids = [m["id"] for m in d["data"]]
    # The remote's models are there...
    assert "llama-3.3-70b" in ids and "mixtral-8x7b" in ids
    # ...and it is a union, not a replacement.
    assert "local-model" in ids or gw.LOCAL_STATE.get("models") == []
    # No duplicates, and nothing says where a model lives -- that is fleet
    # composition, and it belongs in the authenticated /api/servers.
    assert len(ids) == len(set(ids))
    assert "box" not in json.dumps(d)
    agent.stop()


def test_discovery_needs_no_key(node):
    """Clients need it before they have one, which is why it was public before
    this change and stays public after it."""
    srv, _, _ = node
    code, d = call(srv, "GET", "/v1/models", https=False)
    assert code == 200 and isinstance(d.get("data"), list)


def test_an_unpromoted_server_s_models_are_not_advertised(node):
    """Advertising them would promise something /v1 then refuses with a 404."""
    srv, store, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") >= 2)
    gw._probe_tunnel("box")

    ids = [m["id"] for m in call(srv, "GET", "/v1/models")[1]["data"]]
    assert "llama-3.3-70b" not in ids
    store.set_pool_member("box", True)
    ids = [m["id"] for m in call(srv, "GET", "/v1/models")[1]["data"]]
    assert "llama-3.3-70b" in ids
    agent.stop()


def test_an_offline_server_s_models_are_not_advertised(node):
    srv, store, target = node
    cred, _ = attach(srv, session("sub-a"))
    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    agent.add_pipe(("127.0.0.1", target.server_address[1]))
    assert wait_for(lambda: gw.POOL.idle("box") >= 2)
    gw._probe_tunnel("box")
    store.set_pool_member("box", True)
    assert "llama-3.3-70b" in [m["id"] for m in call(srv, "GET", "/v1/models")[1]["data"]]

    agent.stop_control()
    assert wait_for(lambda: (gw.AGENT_STATE.get("box") or {}).get("state") == "offline")
    assert "llama-3.3-70b" not in [m["id"] for m in
                                   call(srv, "GET", "/v1/models")[1]["data"]]
    agent.stop()


def test_a_pinned_remote_model_list_is_cleaned_on_the_way_out(node):
    """llama.cpp's own list carries each child's argv, including the path to its
    API key file. This node has always stripped its own; relaying someone else's
    raw was the same disclosure with an extra hop."""
    srv, store, _ = node

    class Leaky(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_GET(self):
            body = json.dumps({"object": "list", "data": [{
                "id": "leaky-7b", "object": "model",
                "status": {"value": "loaded",
                           "args": ["--api-key-file", "/run/secrets/apikey"]}}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    leaky = ThreadingHTTPServer(("127.0.0.1", 0), Leaky)
    threading.Thread(target=leaky.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    try:
        attach(srv, session("sub-a"), "leaky", kind="static",
               base_url=f"http://127.0.0.1:{leaky.server_address[1]}/v1")
        user, _ = store.mint("sub-a", label="probe")
        import http.client
        c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
        c.request("GET", "/u/leaky/v1/models", None,
                  {"Authorization": "Bearer " + user, "X-Forwarded-Proto": "https"})
        r = c.getresponse()
        raw = r.read().decode()
        c.close()
        assert r.status == 200
        assert "leaky-7b" in raw
        assert "apikey" not in raw and "args" not in raw and "status" not in raw
    finally:
        leaky.shutdown()


# --- capabilities: telemetry must not become a probe storm -------------------

def _said(sid, msg, monkeypatch):
    """Drive _agent_said with the model probe counted rather than run.

    Called unbound: the method touches only module state, so this exercises the
    real dispatch without standing up a socket for it.
    """
    fired = []
    monkeypatch.setattr(gw.threading, "Thread",
                        lambda *a, **k: type("T", (), {"start": lambda s: fired.append(k)})())
    gw.Handler._agent_said(object(), sid, json.dumps(msg).encode())
    return fired


def test_a_hello_asks_what_the_server_now_serves(monkeypatch):
    gw.AGENT_STATE.pop("box", None)
    fired = _said("box", {"type": "hello", "server_id": "box", "slots": 2},
                 monkeypatch)
    assert len(fired) == 1, "a reconnect must re-learn the model list at once"
    assert gw.AGENT_STATE["box"]["slots"] == 2


def test_periodic_capabilities_do_not_re_probe(monkeypatch):
    """Capabilities arrive every 20 s for as long as the agent is attached.
    Probing on each would spawn a thread and TAKE A PIPE three times a minute,
    forever, to re-answer a question whose answer had not changed.
    """
    gw.AGENT_STATE.pop("box", None)
    cards = [{"index": 0, "name": "NVIDIA RTX 4090", "util_pct": 97}]
    fired = _said("box", {"type": "capabilities", "cards": cards}, monkeypatch)
    assert fired == [], "a capabilities report must not trigger a model probe"
    assert gw.AGENT_STATE["box"]["cards"] == cards
    assert gw.AGENT_STATE["box"]["state"] == "online"


def test_hardware_change_refreshes_live_scheduler_cache(monkeypatch, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    now = time.time()
    store.record_usage({
        "ts": now - 20, "key_id": "k", "sub": "sub-a", "model": "m1",
        "upstream": "box", "prompt_tokens": 1000, "completion_tokens": 10,
        "prompt_ms": 1000, "predicted_ms": 100, "status_code": 200,
    })
    assert store.observe_hardware("box", "v1:old", now=now - 10) is False
    gw._refresh_throughput()
    assert ("box", "m1") in gw.THROUGHPUT

    _said("box", {
        "type": "capabilities", "hardware_fingerprint": "v1:new", "cards": []
    }, monkeypatch)
    _said("box", {
        "type": "capabilities", "hardware_fingerprint": "v1:new", "cards": []
    }, monkeypatch)
    _said("box", {
        "type": "capabilities", "hardware_fingerprint": "", "cards": []
    }, monkeypatch)

    assert ("box", "m1") not in gw.THROUGHPUT


def test_hardware_change_evicts_old_cache_when_fresh_scan_fails(monkeypatch, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    now = time.time()
    store.record_usage({
        "ts": now - 20, "key_id": "k", "sub": "sub-a", "model": "m1",
        "upstream": "box", "prompt_tokens": 1000, "completion_tokens": 10,
        "prompt_ms": 1000, "predicted_ms": 100, "status_code": 200,
    })
    assert store.observe_hardware("box", "v1:old", now=now - 10) is False
    gw._refresh_throughput()
    assert ("box", "m1") in gw.THROUGHPUT

    def failed_scan():
        raise RuntimeError("scan failed")

    monkeypatch.setattr(store, "throughput", failed_scan)

    gw.Handler._agent_said(object(), "box", json.dumps({
        "type": "capabilities", "hardware_fingerprint": "v1:new", "cards": []
    }).encode())

    assert ("box", "m1") not in gw._throughput_snapshot()


def test_hardware_invalidation_cannot_be_overwritten_by_older_refresh(
        monkeypatch, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    now = time.time()
    store.record_usage({
        "ts": now - 20, "key_id": "k", "sub": "sub-a", "model": "m1",
        "upstream": "box", "prompt_tokens": 1000, "completion_tokens": 10,
        "prompt_ms": 1000, "predicted_ms": 100, "status_code": 200,
    })
    assert store.observe_hardware("box", "v1:old", now=now - 10) is False
    original_throughput = store.throughput
    first_snapshot = threading.Event()
    second_query = threading.Event()
    release_first = threading.Event()
    call_lock = threading.Lock()
    calls = 0

    def delayed_throughput():
        nonlocal calls
        result = original_throughput(now=now + 1)
        with call_lock:
            calls += 1
            call_number = calls
        if call_number == 1:
            first_snapshot.set()
            assert release_first.wait(3)
        else:
            second_query.set()
        return result

    monkeypatch.setattr(store, "throughput", delayed_throughput)
    stale = threading.Thread(target=gw._refresh_throughput, daemon=True)
    stale.start()
    assert first_snapshot.wait(3)
    assert store.observe_hardware("box", "v1:new", now=now) is True

    fresh = threading.Thread(target=gw._refresh_throughput, daemon=True)
    fresh.start()
    second_query.wait(0.2)
    release_first.set()
    stale.join(3)
    fresh.join(3)

    assert not stale.is_alive() and not fresh.is_alive()
    assert ("box", "m1") not in gw.THROUGHPUT


def test_planner_cannot_read_precutoff_cache_while_fresh_scan_is_blocked(
        monkeypatch, node):
    srv, store, _target = node
    attach(srv, session("sub-a"), "box")
    now = time.time()
    store.record_usage({
        "ts": now - 20, "key_id": "k", "sub": "sub-a", "model": "m1",
        "upstream": "box", "prompt_tokens": 1000, "completion_tokens": 10,
        "prompt_ms": 1000, "predicted_ms": 100, "status_code": 200,
    })
    assert store.observe_hardware("box", "v1:old", now=now - 10) is False
    gw._refresh_throughput()
    assert ("box", "m1") in gw.THROUGHPUT

    original_throughput = store.throughput
    scan_started = threading.Event()
    release_scan = threading.Event()

    def blocked_fresh_scan():
        scan_started.set()
        assert release_scan.wait(3)
        return original_throughput(now=now + 1)

    monkeypatch.setattr(store, "throughput", blocked_fresh_scan)
    invalidator = threading.Thread(
        target=gw.Handler._agent_said,
        args=(object(), "box", json.dumps({
            "type": "capabilities", "hardware_fingerprint": "v1:new", "cards": []
        }).encode()), daemon=True)
    invalidator.start()
    assert scan_started.wait(3)

    captured = []
    planner_attempted = threading.Event()
    planner_done = threading.Event()
    planner_errors = []
    real_snapshot = gw._throughput_snapshot

    def attempted_snapshot():
        planner_attempted.set()
        return real_snapshot()

    monkeypatch.setattr(gw, "_throughput_snapshot", attempted_snapshot)

    class Planner:
        headers = {}
        _body_left = 1

        def _peek_body(self):
            return "m1", True

        def _last_server(self):
            return None

        def _local_online(self):
            return True

        def _load(self, _servers):
            return {}

        def _ready(self, _servers):
            return {}

        def _prompt_tokens(self):
            return 1

        def _decided(self, _server, _why):
            pass

    monkeypatch.setattr(gw, "_servers_all", lambda: [])
    monkeypatch.setattr(gw, "_state_view", lambda _servers: {})

    def pick(_servers, _state, _last, **kwargs):
        captured.append(kwargs["stats"])
        return gw._ups.LOCAL, "measured", 0.0

    monkeypatch.setattr(gw._ups, "pick_auto", pick)

    def plan():
        try:
            gw.Handler._balance(Planner(), "/v1/chat/completions")
        except Exception as exc:
            planner_errors.append(exc)
        finally:
            planner_done.set()

    planner = threading.Thread(target=plan, daemon=True)
    planner.start()
    assert planner_attempted.wait(3), "the planner never reached its cache snapshot"
    blocked = not planner_done.wait(0.2)
    release_scan.set()
    invalidator.join(3)
    planner.join(3)

    assert blocked, "the planner read pre-cutoff THROUGHPUT while refresh was blocked"
    assert not invalidator.is_alive() and not planner.is_alive()
    assert planner_errors == []
    assert len(captured) == 1 and ("box", "m1") not in captured[0]


def test_a_reported_card_list_is_bounded(monkeypatch):
    """It arrives from another machine. An unbounded list is a memory hole."""
    gw.AGENT_STATE.pop("box", None)
    _said("box", {"type": "capabilities",
                  "cards": [{"index": i} for i in range(500)]}, monkeypatch)
    assert len(gw.AGENT_STATE["box"]["cards"]) == 16


def test_junk_where_cards_should_be_is_ignored(monkeypatch):
    gw.AGENT_STATE.pop("box", None)
    _said("box", {"type": "capabilities", "cards": "all of them"}, monkeypatch)
    assert "cards" not in gw.AGENT_STATE["box"]
    _said("box", {"type": "capabilities", "cards": [1, 2, {"index": 0}]}, monkeypatch)
    assert gw.AGENT_STATE["box"]["cards"] == [{"index": 0}]
