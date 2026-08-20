"""Inference through a tunnelled server: the default route over a held-open pipe.

These assert what the transport was built for, against a real target behind a real
fake agent:

  * a request routed to a tunnelled server ARRIVES there, byte-identical, even at
    the measured 469 KB request size;
  * capacity is finite in a way it never was for a direct server -- when no pipe
    is free the balanced route goes AROUND the box, while a pin gets an honest
    503 rather than a hang;
  * promotion is what admits a server to the default route, and a pin reaches it
    either way.
"""
import http.client
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from conftest import load_script
from fakeagent import FakeAgent

gw = load_script("gateway")

REMOTE_ONLY = "llama-3.3-70b"       # deliberately not a qwen id
LOCAL_ONLY = "local-model"


class Target(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        body = json.dumps({"object": "list",
                           "data": [{"id": REMOTE_ONLY}]}).encode()
        self._send(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        self.server.seen.append(self.rfile.read(n))
        self._send(json.dumps({
            "model": REMOTE_ONLY,
            "usage": {"prompt_tokens": 11, "completion_tokens": 3}}).encode())

    def _send(self, body):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


class LocalRuntime(Target):
    """Stands in for this node's own llama.cpp."""

    def do_GET(self):
        if self.path == "/health":
            return self._send(b'{"status":"ok"}')
        self._send(json.dumps({"data": [{"id": LOCAL_ONLY}]}).encode())

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        self.server.seen.append(self.rfile.read(n))
        self._send(json.dumps({"model": LOCAL_ONLY,
                               "usage": {"prompt_tokens": 7,
                                         "completion_tokens": 2}}).encode())


def _serve(handler):
    srv = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    srv.seen = []
    threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()
    return srv


def wait_for(predicate, seconds=3.0):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.02)
    return False


@pytest.fixture
def fleet(tmp_path):
    from keystore import KeyStore
    runtime, target = _serve(LocalRuntime), _serve(Target)

    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    store.upsert_user("sub-a", email="a@example.org", name="A")
    user_key, _ = store.mint("sub-a", label="k")
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))

    gw.STORE = store
    gw.INTERNAL_KEY = "internal"
    gw.CFG.upstream_host = "127.0.0.1"
    gw.CFG.upstream_port = runtime.server_address[1]
    gw.CFG.user_group = "*"
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
    gw.LOCAL_STATE.update(state="online", models=[LOCAL_ONLY], error=None,
                          last_seen=time.time())

    srv = ThreadingHTTPServer(("127.0.0.1", 0), gw.Handler)
    threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                     daemon=True).start()

    agent = FakeAgent(srv.server_address[1], cred)
    agent.control("box")
    yield srv, store, agent, target, runtime, user_key
    agent.stop()
    srv.shutdown()
    runtime.shutdown()
    target.shutdown()


def promote(store, on=True):
    store.set_pool_member("box", on)


def offer(agent, target, n=1):
    for _ in range(n):
        assert agent.add_pipe(("127.0.0.1", target.server_address[1])) == 101
    assert wait_for(lambda: gw.POOL.idle("box") >= n)


def probe(agent, target):
    """Let the node learn what the tunnelled server serves, as the timer would."""
    offer(agent, target)
    gw._probe_tunnel("box")
    assert gw.UP_STATE["box"]["models"] == [REMOTE_ONLY]


def infer(srv, key, model, path="/v1/chat/completions", pad=0):
    body = json.dumps({"model": model,
                       "messages": [{"role": "user", "content": "x" * pad}]}).encode()
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=30)
    c.request("POST", path, body,
              {"Authorization": "Bearer " + key, "Content-Type": "application/json",
               "Content-Length": str(len(body))})
    r = c.getresponse()
    payload = r.read()
    out = (r.status, r.getheader("X-Routed-To"), r.getheader("X-Routed-Why"),
           r.getheader("X-Routed-Est"), payload)
    c.close()
    return out


# --- inference through a pipe ------------------------------------------------

def test_the_default_route_reaches_a_tunnelled_server(fleet):
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    offer(agent, target)
    status, to, why, est, body = infer(srv, key, REMOTE_ONLY)
    assert (status, to) == (200, "box")
    assert json.loads(body)["model"] == REMOTE_ONLY     # really served there
    assert len(target.seen) == 1 and not runtime.seen
    assert est is not None                              # the prediction rides out


def test_a_469_kb_body_reaches_the_tunnelled_target_byte_identical(fleet):
    """The measured real request size at the 100k tier, through a WebSocket."""
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    offer(agent, target)
    status, to, _why, _est, _ = infer(srv, key, REMOTE_ONLY, pad=469 * 1024)
    assert (status, to) == (200, "box")
    sent = target.seen[-1]
    assert len(sent) > 469 * 1024
    assert json.loads(sent)["model"] == REMOTE_ONLY
    assert sent.count(b"x") == 469 * 1024


def test_usage_is_attributed_to_the_tunnelled_server(fleet):
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    offer(agent, target)
    infer(srv, key, REMOTE_ONLY)
    deadline = time.time() + 2
    rows = []
    while time.time() < deadline and not rows:
        with store._conn() as c:
            rows = [dict(r) for r in c.execute(
                "SELECT upstream, model, prompt_tokens FROM usage_events")]
        time.sleep(0.02)
    assert rows and rows[0]["upstream"] == "box"
    assert rows[0]["prompt_tokens"] == 11        # from the model's own response


def test_a_finished_request_frees_its_pipe_slot(fleet):
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    offer(agent, target)
    infer(srv, key, REMOTE_ONLY)
    # One request per pipe: the pipe is spent, and the accounting has to say so
    # or the server looks permanently busy.
    assert wait_for(lambda: gw.POOL.in_flight("box") == 0)


# --- promotion --------------------------------------------------------------

def test_an_unpromoted_server_is_not_in_the_default_route_but_a_pin_reaches_it(fleet):
    """Self-service to attach, admin to admit. Both halves asserted, because a
    gate that blocks everything is not a gate."""
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    offer(agent, target)
    status, _to, _why, _est, body = infer(srv, key, REMOTE_ONLY)
    assert status == 404
    err = json.loads(body)["error"]
    # The refusal must not blame the server for being offline -- it is answering.
    # And it says how to reach it, because the caller may well be its owner.
    assert err["type"] == "model_not_in_pool"
    assert "/u/box/v1" in err["message"]
    assert not target.seen

    status, to, why, _est, _ = infer(srv, key, REMOTE_ONLY,
                                     path="/u/box/v1/chat/completions")
    assert (status, to, why) == (200, "box", "pinned")
    assert len(target.seen) == 1


def test_promotion_admits_it_to_the_default_route(fleet):
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    offer(agent, target)
    assert infer(srv, key, REMOTE_ONLY)[1] == "box"


# --- capacity ---------------------------------------------------------------

def test_a_pin_with_no_free_pipe_is_refused_honestly(fleet, monkeypatch):
    """The node cannot invent a socket. Waiting forever would be a hang; 502
    would blame the server for being busy."""
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)          # consumes the pipe it offered
    monkeypatch.setattr(gw, "PIPE_WAIT_SECONDS", 0.3)
    started = time.time()
    status, _to, why, _est, body = infer(srv, key, REMOTE_ONLY,
                                        path="/u/box/v1/chat/completions")
    assert status == 503
    assert json.loads(body)["error"]["type"] == "no_capacity"
    assert 0.25 < time.time() - started < 5.0
    assert gw.ROUTE_WHY.get("no-capacity") == 1


def test_the_balanced_route_goes_around_a_server_with_no_free_pipe(fleet, monkeypatch):
    """Ranked last, not excluded: /v1 keeps working while the box is saturated."""
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    monkeypatch.setattr(gw, "PIPE_WAIT_SECONDS", 0.3)
    # Both can serve this one, and the tunnelled server has nothing free.
    gw.UP_STATE["box"]["models"] = [REMOTE_ONLY, LOCAL_ONLY]
    status, to, _why, _est, _ = infer(srv, key, LOCAL_ONLY)
    assert (status, to) == (200, "local")
    assert len(runtime.seen) == 1 and not target.seen


def test_a_model_only_the_saturated_server_has_is_still_attempted(fleet, monkeypatch):
    """Ranking it last must not become excluding it: if it is the only server
    that has the model, the request goes there and waits for a pipe."""
    srv, store, agent, target, runtime, key = fleet
    probe(agent, target)
    promote(store)
    monkeypatch.setattr(gw, "PIPE_WAIT_SECONDS", 0.4)

    def late_pipe():
        time.sleep(0.15)
        agent.add_pipe(("127.0.0.1", target.server_address[1]))

    threading.Thread(target=late_pipe, daemon=True).start()
    status, to, _why, _est, _ = infer(srv, key, REMOTE_ONLY)
    assert (status, to) == (200, "box")
