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

from conftest import load_script
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
