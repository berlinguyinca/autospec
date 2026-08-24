"""HEAD, on both listeners, over real sockets.

Neither handler defined `do_HEAD`, so BaseHTTPRequestHandler answered 501 --
measured against the live node on every public path at once: `/`, `/status`,
`/api/queue`, `/api/stats`, `/api/gateway-health`, `/v1/models`, `/api/me`. An
uptime monitor issuing HEAD, which is the conventional cheap probe, reported a
healthy node as broken.

The responses are parsed BY HAND rather than through http.client, because
http.client is told a HEAD carries no body and skips reading one. It would
therefore pass a handler that sent a body anyway -- and a stray body is not a
cosmetic fault: it is read as the beginning of the NEXT response on a keep-alive
connection, which is the class of bug this node has now shipped twice (an
undrained request body, then an unread response body). So every test here checks
BOTH halves: the headers a GET would have sent, and not one byte more.

Two failure modes drive the rest:

  * The flag that suppresses the body lives on the handler INSTANCE, and one
    instance serves every request on a keep-alive connection. Left set, it
    silences the next real GET -- headers arriving, bodies never. Hence a
    HEAD-then-GET-on-one-connection test for each listener.
  * A HEAD relayed to inference would take a pipe from the tunnel pool, move the
    caller's prefix-cache affinity and write a usage row, to collect a refusal
    from a runtime that registers Get and Post and nothing else. Hence the
    405 tests, which assert the side effects did NOT happen.
"""
import json
import socket
import threading
from http.server import ThreadingHTTPServer

import pytest

from nodescripts import load_script

gw = load_script("gateway")
dash = load_script("dashboard")
keys = load_script("keys")


class Conn:
    """One connection, raw, with a hand-rolled response reader."""

    def __init__(self, port, timeout=6.0):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        self.buf = b""

    def send(self, method, path, headers=()):
        req = f"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
        for k, v in headers:
            req += f"{k}: {v}\r\n"
        self.s.sendall((req + "\r\n").encode())

    def _fill(self):
        b = self.s.recv(65536)
        if not b:
            raise EOFError("the server closed before answering")
        self.buf += b

    def response(self, with_body=True):
        """Status, headers, body. `with_body=False` for a HEAD: nothing is
        consumed as a body, so anything the handler wrongly sent stays in the
        buffer where `quiet()` will find it."""
        while b"\r\n\r\n" not in self.buf:
            self._fill()
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        lines = head.decode("latin-1").split("\r\n")
        status = int(lines[0].split()[1])
        hdrs = {}
        for ln in lines[1:]:
            k, _, v = ln.partition(":")
            hdrs[k.strip().lower()] = v.strip()
        body = b""
        if with_body:
            n = int(hdrs.get("content-length") or 0)
            while len(self.buf) < n:
                self._fill()
            body, self.buf = self.buf[:n], self.buf[n:]
        return status, hdrs, body

    def quiet(self, seconds=0.4):
        """Did the server send nothing further? A HEAD that leaked its body
        fails here, and would corrupt the next response if it were allowed to."""
        self.s.settimeout(seconds)
        try:
            while True:
                b = self.s.recv(65536)
                if not b:
                    break
                self.buf += b
        except (socket.timeout, TimeoutError, OSError):
            pass
        return self.buf == b""

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def head_matches_get(port, path, headers=()):
    """A HEAD's status and Content-Length equal the GET's, with no body.

    Content-Length must be the REAL length -- the body is built and dropped, not
    shortened to zero, or a caller sizing a download from the header is misled.
    """
    g = Conn(port)
    g.send("GET", path, headers)
    gs, gh, gbody = g.response()
    g.close()

    h = Conn(port)
    h.send("HEAD", path, headers)
    hs, hh, _ = h.response(with_body=False)
    silent = h.quiet()
    h.close()

    return {"status": (gs, hs), "length": (gh.get("content-length"),
                                           hh.get("content-length")),
            "get_bytes": len(gbody), "silent": silent, "head_headers": hh}


# --- the gateway ------------------------------------------------------------

@pytest.fixture
def node(tmp_path):
    from keystore import KeyStore
    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    gw.STORE = store
    gw.CFG.user_group = "*"
    gw.LAST_SERVER.clear()
    srv = ThreadingHTTPServer(("127.0.0.1", 0), gw.Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield srv.server_address[1], store
    srv.shutdown()


def test_the_gateway_answers_head_instead_of_501(node):
    port, _ = node
    r = head_matches_get(port, "/api/gateway-health")
    assert r["status"] == (200, 200)
    assert r["get_bytes"] > 0                      # a GET really does have a body


def test_a_gateway_head_reports_the_length_a_get_would_send(node):
    port, _ = node
    r = head_matches_get(port, "/api/gateway-health")
    assert r["length"][0] == r["length"][1]
    assert int(r["length"][1]) == r["get_bytes"]


def test_a_gateway_head_sends_no_body_at_all(node):
    port, _ = node
    assert head_matches_get(port, "/api/gateway-health")["silent"]


def test_head_works_on_the_identity_endpoint_too(node):
    port, _ = node
    r = head_matches_get(port, "/api/me")
    assert r["status"] == (200, 200)
    assert r["length"][0] == r["length"][1]
    assert r["silent"]


def test_a_gateway_head_does_not_silence_the_next_get_on_the_connection(node):
    """The regression test for the suppression flag's lifetime. One handler
    instance serves both requests; if the flag survives the HEAD, this GET
    arrives as headers with no body -- the exact hang this dashboard had."""
    port, _ = node
    c = Conn(port)
    c.send("HEAD", "/api/gateway-health")
    hs, _, _ = c.response(with_body=False)
    c.send("GET", "/api/gateway-health")
    gs, gh, gbody = c.response()
    c.close()
    assert (hs, gs) == (200, 200)
    assert len(gbody) == int(gh["content-length"]) > 0
    assert json.loads(gbody)["login_configured"] in (True, False)


def test_two_heads_in_a_row_on_one_connection_both_answer(node):
    port, _ = node
    c = Conn(port)
    c.send("HEAD", "/api/gateway-health")
    first, _, _ = c.response(with_body=False)
    c.send("HEAD", "/api/me")
    second, _, _ = c.response(with_body=False)
    c.close()
    assert (first, second) == (200, 200)


def test_a_head_is_refused_on_the_inference_path(node):
    """405 with an Allow header, not a relayed request. RFC 9110 requires the
    Allow, and a client library reads it to learn what the endpoint takes."""
    port, _ = node
    c = Conn(port)
    c.send("HEAD", "/v1/chat/completions")
    status, hdrs, _ = c.response(with_body=False)
    silent = c.quiet()
    c.close()
    assert status == 405
    assert hdrs["allow"] == "GET, POST"
    assert int(hdrs["content-length"]) > 0     # the length of the error a GET sees
    assert silent


def test_the_inference_refusal_costs_no_pipe_and_no_affinity(node):
    """The refusal happens above authentication and above routing, so a probe
    cannot take a tunnel pipe, move a warm-cache affinity or bill anybody."""
    port, store = node
    presented, _ = store.mint("someone", "probe")
    before = dict(gw.LAST_SERVER)
    c = Conn(port)
    c.send("HEAD", "/v1/chat/completions",
           [("Authorization", f"Bearer {presented}")])
    status, _, _ = c.response(with_body=False)
    c.close()
    assert status == 405                     # the method, not the credential
    assert dict(gw.LAST_SERVER) == before
    assert store.usage(days=365) == []


def test_a_head_on_a_pinned_path_is_refused_rather_than_relayed(node):
    port, _ = node
    c = Conn(port)
    c.send("HEAD", "/u/local/v1/chat/completions")
    status, hdrs, _ = c.response(with_body=False)
    c.close()
    assert status == 405
    assert "allow" in hdrs


def test_a_head_cannot_reach_the_websocket_endpoints(node):
    """These hijack the socket and then block reading frames. Reached by a HEAD
    they would strand a thread against a client that is not speaking WebSocket,
    so they must fall through to 404 -- and must do it at once."""
    port, _ = node
    for path in ("/api/agent/control", "/api/agent/pipe"):
        c = Conn(port, timeout=3.0)
        c.send("HEAD", path)
        status, _, _ = c.response(with_body=False)
        c.close()
        assert status == 404, path


# --- the dashboard ----------------------------------------------------------

@pytest.fixture
def board():
    dash._CACHE.clear()
    dash._CACHE.update({"llama_up": True, "queue": {"slots": 2, "processing": 0,
                                                   "queued": 0, "samples": 0,
                                                   "completions": 0}})
    dash.Handler.api_key = "secret"
    srv = ThreadingHTTPServer(("127.0.0.1", 0), dash.Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield srv.server_address[1]
    srv.shutdown()
    dash.Handler.api_key = None


def test_the_dashboard_answers_head_on_the_page(board):
    r = head_matches_get(board, "/")
    assert r["status"] == (200, 200)
    assert r["get_bytes"] > 0
    assert r["length"][0] == r["length"][1]
    assert r["silent"]


def test_the_dashboard_answers_head_on_the_public_queue(board):
    r = head_matches_get(board, "/api/queue")
    assert r["status"] == (200, 200)
    assert int(r["length"][1]) == r["get_bytes"]
    assert r["silent"]


def test_the_dashboard_answers_head_on_the_status_page(board):
    r = head_matches_get(board, "/status")
    assert r["status"] == (200, 200)
    assert r["silent"]


def test_a_head_on_the_dashboard_still_needs_the_key_where_a_get_does(board):
    """HEAD must not become a way to read what a GET may not. The refusal keeps
    its real Content-Length and sends no body."""
    r = head_matches_get(board, "/api/stats")
    assert r["status"] == (401, 401)
    assert r["length"][0] == r["length"][1]
    assert r["silent"]


def test_the_dashboard_head_authorises_the_same_key_a_get_does(board):
    r = head_matches_get(board, "/api/stats",
                         [("Authorization", "Bearer secret")])
    assert r["status"] == (200, 200)
    assert r["silent"]


def test_an_unknown_dashboard_path_is_404_on_head_as_on_get(board):
    r = head_matches_get(board, "/nope")
    assert r["status"] == (404, 404)
    assert r["silent"]


def test_the_dashboard_closes_after_a_head_rather_than_reusing_it(board):
    """The gateway's flag-lifetime hazard -- one handler instance serving a HEAD
    and then a GET on the same connection -- cannot arise here, and this pins
    the reason: this listener speaks HTTP/1.0 and closes after every response,
    so ThreadingHTTPServer builds a fresh handler for the next one.

    Asserted rather than assumed, because if this listener is ever moved to
    HTTP/1.1 for keep-alive, the hazard becomes real and this test is where that
    change announces itself.
    """
    c = Conn(board)
    c.send("HEAD", "/api/queue")
    status, _, _ = c.response(with_body=False)
    assert status == 200
    with pytest.raises(EOFError):
        c.send("GET", "/api/queue")
        c.response()
    c.close()


def test_a_get_after_a_head_still_carries_its_body(board):
    """The property the flag is there to protect, through the reader's own
    interface: whatever the HEAD did, the next GET is complete."""
    h = Conn(board)
    h.send("HEAD", "/api/queue")
    h.response(with_body=False)
    h.close()

    g = Conn(board)
    g.send("GET", "/api/queue")
    status, hdrs, body = g.response()
    g.close()
    assert status == 200
    assert len(body) == int(hdrs["content-length"]) > 0
    assert "slots" in json.loads(body)


def test_the_auth_request_header_endpoint_is_unaffected(board):
    """It already answered 204 with no body, and nginx issues its own GET
    subrequest regardless of the parent method -- so it must keep working
    exactly as before."""
    c = Conn(board)
    c.send("GET", "/api/queue-headers")
    status, hdrs, body = c.response()
    c.close()
    assert status == 204
    assert body == b""
    assert hdrs["x-queue-slots"] == "2"
