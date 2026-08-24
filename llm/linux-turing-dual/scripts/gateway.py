#!/usr/bin/env python3
"""The authenticating gateway: per-user keys in, exact usage out.

WHERE THIS SITS

    client -> nginx -> THIS (127.0.0.1) -> llama.cpp (127.0.0.1)
                                     `-> another GPU server

THE DEFAULT ROUTE IS THE VIRTUAL SERVER. `/v1` does not mean "this machine", it
means "whichever machine should serve this" -- so every client already pointed at
this node is load-balanced without touching its configuration. `/u/local/v1`
pins this machine and `/u/<id>/v1` pins a named one.

and why it is a separate process from the dashboard: the two have OPPOSITE
correct behaviour when they break. The dashboard's queue snapshot is telemetry,
so losing it must never cost service -- nginx deliberately fails open around it.
Authentication is the reverse: losing it must never GRANT service. Those cannot
share a failure domain.

FIVE PROPERTIES THIS FILE MUST KEEP

 1. Neither direction is buffered. Bodies are relayed in bounded chunks and
    response chunks are flushed as they arrive. A 100k prompt is ~400 KB and its
    prefill takes minutes; buffering either way would add that to every request.
 2. No admission control and no request queue. Concurrency stays bounded by the
    runtime's slots, so the dashboard's existing queue arithmetic stays true. A
    second queue here would make that panel lie.
 3. The client's credential is never forwarded upstream. llama.cpp holds its own
    internal key, which only this process knows.
 4. Token counts come from the response. The request body is PEEKED for routing
    -- a bounded 8 KB prefix, to learn which model was asked for -- and never
    parsed for accounting. See modelpeek.py for why that peek is unavoidable:
    llama.cpp answers a request for a model it has not got with whatever it HAS
    got, so a balancer that cannot read the model cannot avoid wrong answers.
 5. Nothing per-user reaches a public payload. This process serves no public
    endpoint at all.
"""
from __future__ import annotations

import argparse
import http.client
import json
import os
import select
import secrets
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from http.cookies import SimpleCookie

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import keys as _keys            # noqa: E402
import modelpeek as _peek       # noqa: E402
import tunnel as _tunnel        # noqa: E402
import wsframe as _ws           # noqa: E402
import oidc as _oidc            # noqa: E402
import upstreams as _ups        # noqa: E402
import usage as _usage          # noqa: E402
import publicview as _public    # noqa: E402
import chat as _chat            # noqa: E402
import health as _health        # noqa: E402
import admission as _adm        # noqa: E402
from keystore import KeyStore   # noqa: E402

CHUNK = 65536
SESSION_COOKIE = "qt_session"
SESSION_TTL = 12 * 3600
STATE_TTL = 600
JWKS_MIN_INTERVAL = 60          # never refetch more often than this
REFRESH_SECONDS = 30            # registry -> mirror; the revocation staleness bound
FLUSH_SECONDS = 30

# Paths this process owns. Everything else is proxied to the runtime.
OWNED = ("/auth/", "/api/keys", "/api/me", "/api/usage", "/api/gateway-health",
         "/api/stats", "/api/servers", "/api/agent/", "/api/chat",
         # Discovery is the fleet's union now, so this process owns it: it is the
         # only one that knows what the other servers serve.
         "/v1/models")
# NO SEPARATE POLL INTERVAL. The housekeeper's own period drives the probes, and
# a second constant beside it was display-only and wrong by 2x -- the Servers
# panel claimed a 60 s probe while the timer ran every 30. One timer, one number.



def owns(path: str) -> bool:
    """Does THIS process answer `path` itself, rather than proxying it?"""
    return any(path == p.rstrip("/") or path.startswith(p) for p in OWNED)


_LOCK = threading.Lock()
_SESSIONS: dict[str, dict] = {}
_PENDING: dict[str, dict] = {}      # oauth state -> {verifier, created}
_JWKS: dict = {}
_JWKS_AT = 0.0


class Config:
    """Site coordinates, all from the environment (which the unit populates from
    site.conf). Nothing here is defaulted to a guess: a missing value must fail
    loudly rather than silently authenticate against the wrong pool."""

    def __init__(self) -> None:
        self.upstream_host = os.environ.get("QT_UPSTREAM_HOST", "127.0.0.1")
        self.upstream_port = int(os.environ.get("QT_UPSTREAM_PORT", "8090"))
        self.dash_port = int(os.environ.get("QT_DASH_PORT_LOCAL", "8081"))
        self.public_fqdn = os.environ.get("QT_PUBLIC_FQDN", "")
        self.region = os.environ.get("QT_COGNITO_REGION", "")
        self.pool_id = os.environ.get("QT_COGNITO_POOL_ID", "")
        self.client_id = os.environ.get("QT_COGNITO_CLIENT_ID", "")
        self.domain = os.environ.get("QT_COGNITO_DOMAIN", "")
        self.user_group = os.environ.get("QT_COGNITO_USER_GROUP", "")
        self.admin_group = os.environ.get("QT_COGNITO_ADMIN_GROUP", "")

    @property
    def redirect_uri(self) -> str:
        return f"https://{self.public_fqdn}/auth/callback"

    @property
    def issuer(self) -> str:
        return _oidc.issuer_url(self.region, self.pool_id)

    def login_configured(self) -> bool:
        return all([self.public_fqdn, self.region, self.pool_id,
                    self.client_id, self.domain])


CFG = Config()
STORE: KeyStore | None = None
INTERNAL_KEY = ""
DASHBOARD_KEY = ""
REGISTRY_PATH = ""
UPSTREAMS: list = []
# id -> {state, last_seen, models, error}. Sampled by the housekeeper, never on
# the request path: probing per request would put another server's latency in
# front of every call.
UP_STATE: dict[str, dict] = {}
UP_KEYS: dict[str, str] = {}
# key_id -> the server it last used, for the affinity that keeps a prefix cache
# warm. Seeded from recorded usage on first miss, so a gateway restart does not
# scatter everybody onto cold slots.
LAST_SERVER: dict[str, str] = {}
# This node's own liveness and model list, from the same kind of probe as a
# remote's -- because the runtime is a SEPARATE process. A live gateway with a
# dead runtime would otherwise read as "local is fine" and pin every balanced
# request to a server that cannot answer.
LOCAL_STATE: dict = {"state": "unknown", "models": [], "error": None,
                     "last_seen": None}
# How balancing decisions came out, by reason. Counted because a balancer that
# quietly stops balancing -- every request going local because the peek keeps
# failing -- looks exactly like a working one from outside.
ROUTE_WHY: dict[str, int] = {}
# Returned by the planner when it has already answered the request.
ANSWERED = object()

# Pipes offered by attached agents, and who is connected.
POOL = _tunnel.PipePool()
# server_id -> {state, last_seen, error, gpus, slots, agent_version}. A tunnelled
# server's liveness is its CONTROL connection, not a probe: the connection either
# exists or it does not, which is a stronger signal than a poll and a faster one.
AGENT_STATE: dict[str, dict] = {}
# server_id -> the thread holding its control connection, so a second connection
# for the same id can be refused rather than silently racing the first.
CONTROL: dict[str, dict] = {}
# Requests in flight per server, including this node as "local". The load term
# the scheduler ranks on.
INFLIGHT: dict[str, int] = {}
# Measured rates, refreshed on the housekeeper's timer. Never computed on the
# request path: it is a scan over usage history.
THROUGHPUT: dict = {}

HEARTBEAT_SECONDS = 20          # ping an agent's control connection this often
HEARTBEAT_GRACE = 10            # and give up on it this long after the last pong
PIPE_KEEPALIVE_SECONDS = 240    # ping idle pipes, well inside nginx's read timeout
PIPE_WAIT_SECONDS = 5           # how long a request waits for a free pipe
# A model list is read whole so it can be cleaned. Bounded because "read whole"
# and "trust the other end about size" must never be the same sentence.
MODEL_LIST_MAX = 1 << 20
# What an agent is asked to keep open. Two slots plus headroom, so a request
# never pays for a TLS handshake it could have prepaid.
DEFAULT_PIPES_WANTED = 4
# How long a request may HOLD for a seat before being told the node is full.
# Sized against measured prefill: ~210 s to the first token at 100k, and nginx's
# read timeout here is 900 s -- so a wait shorter than a legitimate prefill would
# refuse work the node was about to do anyway, and one longer than nginx's would
# be a wait nobody is left to receive.
ADMIT_WAIT_SECONDS = 300
ADMIT = _adm.Admission()
# Model context and slot counts, from the dashboard's catalog. Refreshed by the
# housekeeper: the pool size is a property of how a model was LOADED, so it is
# read from the node rather than assumed.
POOL_SIZES: dict = {}


def _ks_enrol_ttl() -> int:
    """The store owns this number; the endpoint only reports it."""
    from keystore import ENROL_TTL_SECONDS
    return int(ENROL_TTL_SECONDS)


class _Hijacked:
    """Stands in for a connection this process has taken over.

    After a WebSocket upgrade the socket is no longer HTTP, but
    BaseHTTPRequestHandler still flushes and closes wfile when the handler
    returns -- against a file we have already closed, which raises and prints a
    traceback from a worker thread. Swapping in this sink is how the framework's
    epilogue becomes harmless.
    """

    # BaseHTTPRequestHandler.finish() reads `.closed` before flushing, so a sink
    # that lacks it merely trades one traceback for another.
    closed = False

    def write(self, data):          # noqa: D102 - a sink
        return len(data)

    def flush(self):
        pass

    def close(self):
        pass


class NoCapacity(Exception):
    """No pipe was free for a tunnelled server within PIPE_WAIT_SECONDS.

    Its own exception because it is not an upstream error: the server is fine and
    simply busy, and answering 502 would send people looking at the wrong box.
    """
# --- background work, never on the request path -----------------------------

def _jwks(force: bool = False) -> dict:
    """Cached JWKS. Refetched at most once a minute, and only when a token
    presents an unknown kid -- key rotation is routine, a fetch per request is
    not."""
    global _JWKS, _JWKS_AT
    with _LOCK:
        fresh = _JWKS and (time.time() - _JWKS_AT) < JWKS_MIN_INTERVAL
        if fresh and not force:
            return _JWKS
    try:
        url = _oidc.jwks_url(CFG.region, CFG.pool_id)
        with urllib.request.urlopen(url, timeout=5) as r:
            doc = json.loads(r.read())
        with _LOCK:
            _JWKS, _JWKS_AT = doc, time.time()
        return doc
    except Exception as exc:
        sys.stderr.write(f"jwks fetch failed: {exc}\n")
        with _LOCK:
            return _JWKS


def _load_registry() -> None:
    """Re-read the registry. Called on a timer so adding a server does not need a
    restart, and so a syntax error in the file cannot take the gateway down --
    the previous registry stays in force and the problem is reported."""
    global UPSTREAMS
    if not REGISTRY_PATH:
        return
    try:
        with open(REGISTRY_PATH) as fh:
            found = _ups.load(fh.read())
    except FileNotFoundError:
        UPSTREAMS = []
        return
    except Exception as exc:
        sys.stderr.write(f"upstream registry unreadable, keeping the previous "
                         f"one: {exc}\n")
        return
    UPSTREAMS = found
    # Credentials for dashboard-attached servers arrive the same way as file ones,
    # into the same map, so the relay never asks which route a key came by.
    if STORE is not None:
        try:
            UP_KEYS.update(STORE.upstream_keys())
        except Exception as exc:
            sys.stderr.write(f"attached-server keys unavailable: {exc}\n")
    for u in found:
        if u.key_file and u.id not in UP_KEYS:
            try:
                UP_KEYS[u.id] = (open(u.key_file).readline() or "").strip()
            except OSError as exc:
                u.problems.append(f"key_file unreadable: {exc.strerror}")


def _servers_all() -> list:
    """File-configured entries and attached ones, as ONE list.

    One list because there is one scheduler and one report. Two lists would mean
    every routing decision asking which kind it was holding, which is how the two
    would drift.

    A file entry wins a name collision: it is on disk, an operator put it there,
    and the attached row can be renamed by whoever owns it.
    """
    out = list(UPSTREAMS)
    if STORE is None:
        return out
    seen = {u.id for u in out}
    try:
        rows = STORE.servers()
    except Exception as exc:
        sys.stderr.write(f"attached servers unavailable: {exc}\n")
        return out
    for u in _ups.from_records(rows):
        if u.id in seen:
            u.problems.append("this name is also in the registry file, which wins")
        out.append(u)
    return out


def _state_view(servers) -> dict:
    """UP_STATE, with a tunnelled server's liveness taken from its CONTROL
    connection rather than from the last poll.

    The whole claim for this transport is that a dropped connection is a faster
    and stronger signal than a probe -- but the panel and the refusals read the
    polled state, so a server that had just vanished still showed `online` for up
    to a poll interval, and one that had just come back still showed `offline`.
    Both were observed live. The model list stays polled, because that genuinely
    is a question only the server can answer.
    """
    view = dict(UP_STATE)
    for u in servers:
        if u.kind != _ups.KIND_TUNNEL:
            continue
        agent = AGENT_STATE.get(u.id) or {}
        row = dict(view.get(u.id) or {})
        row["state"] = agent.get("state", "unknown")
        row["last_seen"] = agent.get("last_seen") or row.get("last_seen")
        row["error"] = agent.get("error") or row.get("error")
        # What the server said about ITSELF in its hello. Carried through here so
        # the panel can report a remote's seat count and agent build: the hello
        # already sent both and they were being dropped on the floor.
        row["slots"] = agent.get("slots") or row.get("slots")
        row["agent_version"] = agent.get("agent_version") or row.get("agent_version")
        row["cards"] = agent.get("cards") or row.get("cards")
        row.setdefault("models", [])
        view[u.id] = row
    return view


def _probe_tunnel(server_id: str) -> None:
    """Ask a tunnelled server what it serves, over one of its own pipes.

    The same question asked of a direct server, down a different socket -- so the
    model list has one source of truth (the server's own /v1/models) regardless of
    how the node reaches it. Nothing here is llama.cpp-specific: any
    OpenAI-compatible server answers this.
    """
    prev = UP_STATE.get(server_id, {})
    agent = AGENT_STATE.get(server_id) or {}
    if agent.get("state") != "online":
        UP_STATE[server_id] = {"state": agent.get("state", "unknown"),
                               "models": prev.get("models") or [],
                               "error": agent.get("error"),
                               "last_seen": agent.get("last_seen")}
        return
    # A short wait on purpose: this runs on a timer, and a server whose pipes are
    # all busy should not have its model list blocked behind real work.
    pipe = POOL.take(server_id, timeout=1.0)
    if pipe is None:
        UP_STATE[server_id] = {"state": "online", "models": prev.get("models") or [],
                               "error": None, "last_seen": agent.get("last_seen")}
        return
    try:
        up = http.client.HTTPConnection(server_id, 80, timeout=10)
        up.sock = pipe
        up.putrequest("GET", "/v1/models", skip_host=True, skip_accept_encoding=True)
        up.putheader("Host", server_id)
        up.endheaders()
        r = up.getresponse()
        body = r.read()
        if r.status != 200:
            raise RuntimeError(f"HTTP {r.status}")
        data = json.loads(body).get("data") or []
        models = [m.get("id") for m in data if isinstance(m, dict) and m.get("id")]
        UP_STATE[server_id] = {"state": "online", "models": models, "error": None,
                               "last_seen": time.time()}
    except Exception as exc:
        UP_STATE[server_id] = {"state": "online", "models": prev.get("models") or [],
                               "error": f"model list unavailable: {str(exc)[:80]}",
                               "last_seen": agent.get("last_seen")}
    finally:
        pipe.close()


def _probe_when_ready(server_id: str, tries: int = 10) -> None:
    """Probe as soon as the agent has a pipe to SPARE.

    Called when a server says hello, which happens before its pipes arrive, so
    this waits rather than probing into an empty pool.

    It requires a spare -- more than one idle pipe -- because a model list is
    telemetry and a request is not. Taking the last pipe for a poll would make a
    real request wait for one, and this whole probe is only an optimisation: the
    30 s timer asks the same question anyway. That rule also made the tests
    deterministic, which is how the race was noticed at all.
    """
    for _ in range(tries):
        if POOL.idle(server_id) > 1:
            try:
                _probe_tunnel(server_id)
            except Exception as exc:
                sys.stderr.write(f"probe of {server_id} failed: {exc}\n")
            return
        time.sleep(0.5)


def _poll_upstreams() -> None:
    """Ask each usable server what it serves.

    `state` is online / offline / unknown -- never assumed online. A server that
    has never answered is `unknown`, which is a different thing from one that
    answered and then stopped.
    """
    for u in _servers_all():
        if u.kind == _ups.KIND_TUNNEL:
            _probe_tunnel(u.id)
            continue
        if not u.usable:
            UP_STATE[u.id] = {"state": "disabled", "models": [],
                              "error": "; ".join(u.problems) or "parked",
                              "last_seen": UP_STATE.get(u.id, {}).get("last_seen")}
            continue
        scheme, host, port, path = _ups.target(u, "/v1/models")
        try:
            cls = (http.client.HTTPSConnection if scheme == "https"
                   else http.client.HTTPConnection)
            conn = cls(host, port, timeout=6)
            headers = {}
            if UP_KEYS.get(u.id):
                headers["Authorization"] = "Bearer " + UP_KEYS[u.id]
            conn.request("GET", path, headers=headers)
            r = conn.getresponse()
            body = r.read()
            conn.close()
            if r.status in (401, 403):
                # It is answering, so it is not offline -- it is refusing US. A
                # server registered from the dashboard cannot carry a credential
                # yet (only a registry-file entry can, via key_file), and without
                # this the symptom is a server that looks online, reports no
                # models, and is quietly ineligible forever.
                UP_STATE[u.id] = {
                    "state": "online", "models": [],
                    "error": (f"this server wants a credential (HTTP {r.status}); "
                              f"register it in the registry file with a key_file, "
                              f"or bind it to loopback and attach it with the agent"),
                    "last_seen": time.time(), "authenticated": False}
                continue
            if r.status != 200:
                raise RuntimeError(f"HTTP {r.status}")
            data = json.loads(body).get("data") or []
            models = [m.get("id") for m in data if isinstance(m, dict) and m.get("id")]
            UP_STATE[u.id] = {"state": "online", "models": models, "error": None,
                              "last_seen": time.time(),
                              "authenticated": bool(UP_KEYS.get(u.id))}
        except Exception as exc:
            prev = UP_STATE.get(u.id, {})
            UP_STATE[u.id] = {"state": "offline", "models": prev.get("models") or [],
                              "error": str(exc)[:120],
                              "last_seen": prev.get("last_seen")}


def _local_problems() -> list:
    """This node's own faults, from the collector that reads its journal.

    Fetched here rather than derived, because the signals are a GPU tool's stderr
    and a systemd journal -- neither of which this process should be forking for.

    It matters that this reaches the FLEET view and not just the node panel:
    llama.cpp's /health answers 200 while CUDA is dead, so `local` sat in the
    fleet as `online` with seven models it could not serve, and every request for
    one of them hung until the caller gave up.
    """
    try:
        conn = http.client.HTTPConnection("127.0.0.1", CFG.dash_port, timeout=5)
        conn.putrequest("GET", "/api/stats", skip_host=True,
                        skip_accept_encoding=True)
        conn.putheader("Host", f"127.0.0.1:{CFG.dash_port}")
        if DASHBOARD_KEY:
            conn.putheader("Authorization", f"Bearer {DASHBOARD_KEY}")
        conn.endheaders()
        r = conn.getresponse()
        body = r.read()
        conn.close()
        if r.status != 200:
            return []
        got = json.loads(body).get("problems")
        return got if isinstance(got, list) else []
    except Exception:
        # Never fatal: losing the fault list must not also lose the routing that
        # depends on this timer. An absent list reads as "nothing known", which is
        # what it is.
        return []


def _poll_local() -> None:
    """Probe the local runtime: is it up, and what does it serve?

    `/health` is the liveness signal and NOT an inference call, so this stays
    off the request path. It was checked against a live model swap before being
    wired into routing: it answered 200 throughout a 7.5 s swap, so a swap does
    not make this node look offline. That matters -- flapping would scatter
    callers onto cold slots, destroying the prefix-cache affinity that balancing
    exists to protect.
    """
    host, port = CFG.upstream_host, CFG.upstream_port
    try:
        conn = http.client.HTTPConnection(host, port, timeout=5)
        conn.request("GET", "/health")
        r = conn.getresponse()
        r.read()
        conn.close()
        # ANY status means the runtime answered, and answering is what liveness
        # asks. Measured on this build: /health, /metrics, /slots and /props all
        # return 401 to every credential, while inference works -- so demanding
        # 200 marked a serving node offline and took it out of the balanced route
        # entirely. Only a transport failure is evidence of death.
        if r.status >= 500:
            raise RuntimeError(f"HTTP {r.status}")
    except Exception as exc:
        # Keep the last known model list: it describes what this node CAN serve,
        # which does not stop being true because the runtime is restarting.
        LOCAL_STATE.update(state="offline", error=str(exc)[:120],
                           problems=_local_problems())
        return

    models = LOCAL_STATE.get("models") or []
    try:
        conn = http.client.HTTPConnection(host, port, timeout=6)
        headers = {"Authorization": "Bearer " + INTERNAL_KEY} if INTERNAL_KEY else {}
        conn.request("GET", "/v1/models", headers=headers)
        r = conn.getresponse()
        body = r.read()
        conn.close()
        if r.status == 200:
            data = json.loads(body).get("data") or []
            # IDS ONLY, and nothing else from this payload ever leaves the
            # process: the runtime's own model list embeds each child's argv,
            # which includes the path to its API key file. That is exactly why
            # the public model list is served sanitised by the dashboard.
            models = [m.get("id") for m in data
                      if isinstance(m, dict) and m.get("id")]
    except Exception as exc:
        sys.stderr.write(f"local model list unavailable: {exc}\n")
    LOCAL_STATE.update(state="online", error=None, models=models,
                       problems=_local_problems(),
                       last_seen=time.time())


def _refresh_throughput() -> None:
    """Recompute measured rates. On a timer because it scans usage history, and
    the request path must never pay for it."""
    global THROUGHPUT
    if STORE is None:
        return
    THROUGHPUT = STORE.throughput()


def _refresh_pool_sizes() -> None:
    """How big each model's KV pool is, and how many slots it was given.

    Read from the node's own catalog rather than assumed, because the pool is a
    property of how the model was LOADED -- a preset edited and reloaded changes
    it, and admission control working from a stale constant would either refuse
    work that fits or admit work that does not.
    """
    try:
        conn = http.client.HTTPConnection("127.0.0.1", CFG.dash_port, timeout=5)
        conn.putrequest("GET", "/api/stats", skip_host=True,
                        skip_accept_encoding=True)
        conn.putheader("Host", f"127.0.0.1:{CFG.dash_port}")
        if DASHBOARD_KEY:
            conn.putheader("Authorization", f"Bearer {DASHBOARD_KEY}")
        conn.endheaders()
        r = conn.getresponse()
        body = r.read()
        conn.close()
        if r.status != 200:
            return
        for m in json.loads(body).get("catalog") or []:
            mid, ctx, slots = m.get("id"), m.get("context"), m.get("slots")
            if mid and ctx:
                POOL_SIZES[mid] = (int(ctx), int(slots or 1))
                for alias in m.get("aliases") or []:
                    # An alias shares the instance's pool -- it is the same loaded
                    # model under another name, which is the whole point of a tier.
                    POOL_SIZES[alias] = (int(ctx), int(slots or 1))
    except Exception:
        return


def _housekeeper() -> None:
    """Registry sync and usage flush. Both are best-effort: a registry outage
    degrades to stale usage, never to a refused request."""
    while True:
        time.sleep(REFRESH_SECONDS)
        try:
            if STORE:
                STORE.refresh()
                STORE.flush()
        except Exception as exc:
            sys.stderr.write(f"housekeeping: {exc}\n")
        try:
            _load_registry()
            _poll_upstreams()
            _poll_local()
        except Exception as exc:
            sys.stderr.write(f"upstream poll: {exc}\n")
        try:
            _refresh_throughput()
        except Exception as exc:
            sys.stderr.write(f"throughput refresh: {exc}\n")
        try:
            _refresh_pool_sizes()
        except Exception as exc:
            sys.stderr.write(f"pool sizes: {exc}\n")
        try:
            # Idle pipes would otherwise be reaped by nginx's read timeout, and
            # the first request after a quiet hour would die on a reset.
            POOL.keepalive()
        except Exception as exc:
            sys.stderr.write(f"pipe keepalive: {exc}\n")
        try:
            now = time.time()
            with _LOCK:
                for sid in [k for k, v in _SESSIONS.items()
                            if now - v["created"] > SESSION_TTL]:
                    _SESSIONS.pop(sid, None)
                for st in [k for k, v in _PENDING.items()
                           if now - v["created"] > STATE_TTL]:
                    _PENDING.pop(st, None)
        except Exception:
            pass


def _verify_identity(id_token: str) -> dict | None:
    """Verify an id_token, retrying ONCE against a forced JWKS refetch.

    The retry exists because provider key rotation is routine: a token signed
    with a new key would otherwise fail until the cache expired. It is bounded to
    one attempt so a stream of forged tokens cannot turn into a fetch storm.
    """
    last = None
    for force in (False, True):
        try:
            return _oidc.verify_id_token(id_token, _jwks(force=force),
                                         issuer=CFG.issuer, audience=CFG.client_id)
        except ValueError as exc:
            last = exc
    sys.stderr.write(f"id_token rejected: {last}\n")
    return None


class Handler(BaseHTTPRequestHandler):
    server_version = "qwen-turing-gateway"
    protocol_version = "HTTP/1.1"
    _auth_key_id = None
    _auth_sub = None
    _auto_choice = None
    _route_why = None
    _route_est = None
    # The prefix of the body already read, to learn the model. Forwarded ahead
    # of the rest of the stream, so peeking costs a buffer and never a re-read.
    _peeked = b""
    # The path with any /u/<server> prefix removed. Set during resolution and
    # used for BOTH destinations: when auto picks this node the prefix still has
    # to go, or the local runtime is asked for a path it has never heard of.
    _route_path = None
    # The model this request named, from the peek. Admission weighs the tier.
    _req_model = None
    # Seconds this request spent holding for a seat. Reported back, because a
    # caller that waited 40 s deserves to know the node was full rather than
    # slow -- those call for different actions.
    _admit_waited = 0.0
    # A HEAD is a GET whose body is thrown away. It is a CLASS attribute rather
    # than per-request state reset in _route(), because _route() runs after
    # do_HEAD() has set it -- resetting it there would clear the flag and send a
    # body on every HEAD.
    _head = False

    # --- helpers -----------------------------------------------------------
    def _read_body(self, n: int) -> bytes:
        """Read at most n bytes of the request body, tracking what is left."""
        if getattr(self, "_body_left", 0) <= 0:
            return b""
        buf = self.rfile.read(min(n, self._body_left))
        self._body_left -= len(buf)
        return buf

    def _drain_body(self) -> None:
        """Consume any unread request body before answering.

        A response that does not read the body leaves it sitting in the socket,
        where the NEXT request on a keep-alive connection parses it as a request
        line. Observed exactly once in testing as

            code 501, Unsupported method ('{"model":...}POST')

        -- a rejected request silently corrupting the good one behind it. Every
        error path must drain, and the counter makes draining idempotent: without
        it, a second drain would block on bytes that are never coming.
        """
        while getattr(self, "_body_left", 0) > 0:
            if not self._read_body(CHUNK):
                self._body_left = 0
                break

    def _body(self, data: bytes) -> None:
        """Write a response body -- unless this is a HEAD, which must carry the
        headers a GET would carry and none of the bytes. The Content-Length is
        therefore still the real length: the body is BUILT and then dropped,
        never shortened to zero, or a monitor reading the header would learn the
        wrong size."""
        if self._head:
            return
        self.wfile.write(data)

    def _json(self, code: int, obj, extra: dict | None = None) -> None:
        body = json.dumps(obj).encode()
        self.send_response(code)
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        # If we are going to close, SAY so. Closing silently gives a client
        # reusing the connection a broken pipe instead of a clean retry.
        if self.close_connection:
            self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self._body(body)

    def _err(self, code: int, message: str, kind: str = "invalid_request",
             extra: dict | None = None) -> None:
        # OpenAI-compatible error shape, so a client library surfaces it.
        self._drain_body()
        self.close_connection = True
        self._json(code, {"error": {"message": message, "type": kind,
                                    "code": code}}, extra)

    def _secure(self) -> bool:
        """Did this request arrive over TLS? nginx sets X-Forwarded-Proto and
        this process binds loopback only, so nothing else can set it."""
        return self.headers.get("X-Forwarded-Proto", "").lower() == "https"

    def _session(self) -> dict | None:
        raw = self.headers.get("Cookie")
        if not raw:
            return None
        try:
            sid = SimpleCookie(raw).get(SESSION_COOKIE)
        except Exception:
            return None
        if not sid:
            return None
        with _LOCK:
            s = _SESSIONS.get(sid.value)
        if not s or time.time() - s["created"] > SESSION_TTL:
            return None
        return s

    def _may(self, s: dict) -> tuple[bool, bool]:
        """(may_mint, is_admin), from the VERIFIED token's groups only.

        QT_COGNITO_USER_GROUP="*" (or empty) means EVERY authenticated member of
        the pool may mint. That is the configured intent here: this node is a
        shared lab resource and the pool is the lab.

        The group mechanism is kept rather than deleted, because re-tightening
        should be a config change rather than a code change -- set the variable
        to a group name and minting narrows to that group again.

        Admin is NOT opened by the same switch. It grants seeing and revoking
        other people's keys, which is a different question from being allowed to
        use the node.
        """
        g = s.get("groups") or []
        is_admin = bool(CFG.admin_group) and CFG.admin_group in g
        open_to_all = CFG.user_group in ("", "*")
        may_mint = is_admin or open_to_all or CFG.user_group in g
        return may_mint, is_admin

    # --- routing -----------------------------------------------------------
    def do_GET(self):
        self._route("GET")

    def do_HEAD(self):
        """The headers a GET would send, and none of the body.

        Without this, BaseHTTPRequestHandler answers 501 -- so every uptime
        monitor pointed at this node reported it broken while it was serving
        happily. Measured before the fix: all seven public paths, 501.

        The flag is cleared in a `finally` because ONE handler instance serves
        every request on a keep-alive connection. Leaving it set would silence
        the body of the next real GET on that connection -- headers arriving and
        bodies never, which is exactly the hang this dashboard just had.
        """
        self._head = True
        try:
            self._route("HEAD")
        finally:
            self._head = False

    def do_POST(self):
        self._route("POST")

    def do_DELETE(self):
        self._route("DELETE")

    def do_PUT(self):
        self._route("PUT")

    def do_OPTIONS(self):
        self._route("OPTIONS")

    def _route(self, method: str) -> None:
        # Body accounting is PER REQUEST, and this handler instance is reused for
        # every request on a keep-alive connection -- so it must be reset here,
        # not in __init__.
        self._body_left = int(self.headers.get("Content-Length") or 0)
        self._peeked = b""
        self._auto_choice = None
        self._route_why = None
        self._route_est = None
        path = self.path.split("?", 1)[0]
        if method == "HEAD" and not owns(path):
            # Only what this process answers itself. A HEAD relayed onward would
            # take a pipe from the tunnel pool, move the caller's cache affinity
            # and write a usage row -- all to collect a refusal from a runtime
            # that registers Get and Post and nothing else. Refused here, above
            # authentication, so an unauthenticated probe costs nothing at all.
            return self._err(405, "HEAD is not supported on this endpoint",
                             "method_not_allowed", {"Allow": "GET, POST"})
        if path.startswith("/u/"):
            return self._proxy(method, remote=True)
        if owns(path):
            try:
                self._owned(method, path)
            except BrokenPipeError:
                pass
            except Exception as exc:
                sys.stderr.write(f"handler error on {path}: {exc}\n")
                try:
                    self._err(500, "internal error")
                except Exception:
                    pass
            return
        self._proxy(method)

    # Exact-match endpoints: (path, method or None for "any method", handler).
    # Adding an endpoint is one row rather than another if-branch, and the
    # dispatcher below is the only code that walks the table. Parameterised
    # routes carry an extraction step and stay in _owned, after this table --
    # the same order the branches ran in before.
    _ROUTES: tuple[tuple[str, str | None, str], ...] = (
        ("/auth/login", "GET", "_login"),
        ("/auth/callback", "GET", "_callback"),
        ("/auth/logout", None, "_logout"),
        ("/api/me", None, "_me"),
        ("/api/gateway-health", None, "_health"),
        ("/api/stats", None, "_stats"),
        ("/v1/models", None, "_models"),
        ("/api/servers", None, "_servers"),
        ("/api/chat", "POST", "_chat"),
        ("/api/servers/enrol", "POST", "_enrol"),
        ("/api/agent/enrol", "POST", "_agent_enrol"),
        ("/api/agent/control", "GET", "_agent_control"),
        ("/api/agent/pipe", "GET", "_agent_pipe"),
        ("/api/usage", None, "_usage_api"),
        ("/api/keys", "GET", "_list_keys"),
        ("/api/keys", "POST", "_mint"),
    )

    def _owned(self, method: str, path: str) -> None:
        for route_path, route_method, handler in self._ROUTES:
            if path == route_path and route_method in (None, method):
                return getattr(self, handler)()
        if path.startswith("/api/servers/"):
            rest = path[len("/api/servers/"):].split("/")
            if len(rest) == 1 and method == "DELETE":
                return self._detach(rest[0])
            if len(rest) == 2 and method == "POST" and rest[1] in ("pool", "priority"):
                return self._server_setting(rest[0], rest[1])
        if path.startswith("/api/keys/") and method == "DELETE":
            return self._revoke(path.rsplit("/", 1)[-1])
        self._err(404, "no such endpoint")

    # --- login -------------------------------------------------------------
    def _login(self) -> None:
        if not CFG.login_configured():
            return self._err(503, "login is not configured on this node")
        verifier, challenge = _oidc.pkce_pair()
        state = secrets.token_urlsafe(24)
        with _LOCK:
            _PENDING[state] = {"verifier": verifier, "created": time.time()}
        url = _oidc.authorize_url(CFG.domain, CFG.client_id, CFG.redirect_uri,
                                  state, challenge)
        self.send_response(302)
        self.send_header("Location", url)
        self.send_header("Content-Length", "0")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()

    def _exchange_code(self, code: str, verifier: str) -> dict | None:
        """Swap the authorization code for tokens. Answers the client and returns
        None on failure, so the caller stays linear."""
        try:
            body = _oidc.token_form(CFG.client_id, code, CFG.redirect_uri, verifier)
            req = urllib.request.Request(
                _oidc.token_url(CFG.domain), data=body,
                headers={"Content-Type": "application/x-www-form-urlencoded"})
            with urllib.request.urlopen(req, timeout=10) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as exc:
            sys.stderr.write(f"token exchange rejected: {exc.read()[:200]!r}\n")
            self._err(400, "token exchange failed")
        except Exception as exc:
            sys.stderr.write(f"token exchange failed: {exc}\n")
            self._err(502, "token exchange failed")
        return None

    def _callback(self) -> None:
        import urllib.parse
        q = urllib.parse.parse_qs(self.path.split("?", 1)[1] if "?" in self.path else "")
        code = (q.get("code") or [""])[0]
        state = (q.get("state") or [""])[0]
        if not code or not state:
            return self._err(400, "missing code or state")
        with _LOCK:
            pending = _PENDING.pop(state, None)
        if not pending:
            # Unknown state: expired, replayed, or forged. All the same answer.
            return self._err(400, "state is not recognised")

        tokens = self._exchange_code(code, pending["verifier"])
        if tokens is None:
            return                      # _exchange_code already answered
        id_token = tokens.get("id_token")
        if not id_token:
            return self._err(502, "provider returned no id_token")
        claims = _verify_identity(id_token)
        if claims is None:
            return self._err(401, "identity token rejected")

        sub, email, name = _oidc.identity(claims)
        groups = _oidc.groups(claims)
        STORE.upsert_user(sub, email, name, login_at=time.time())

        sid = secrets.token_urlsafe(32)
        with _LOCK:
            _SESSIONS[sid] = {"sub": sub, "email": email, "name": name,
                              "groups": groups, "created": time.time()}
        cookie = (f"{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Lax"
                  + ("; Secure" if self._secure() else ""))
        self.send_response(302)
        self.send_header("Set-Cookie", cookie)
        self.send_header("Location", "/")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _logout(self) -> None:
        raw = self.headers.get("Cookie")
        if raw:
            try:
                sid = SimpleCookie(raw).get(SESSION_COOKIE)
                if sid:
                    with _LOCK:
                        _SESSIONS.pop(sid.value, None)
            except Exception:
                pass
        self.send_response(200)
        self.send_header("Set-Cookie",
                         f"{SESSION_COOKIE}=; Path=/; HttpOnly; Max-Age=0")
        self.send_header("Content-Length", "0")
        self.end_headers()

    # --- identity ----------------------------------------------------------
    def _me(self) -> None:
        s = self._session()
        if not s:
            return self._json(200, {"authenticated": False,
                                    "login_configured": CFG.login_configured()})
        may_mint, is_admin = self._may(s)
        # A user in NEITHER group is authenticated and authorised for nothing.
        # That is a distinct state from "not logged in", and the page must be
        # able to say which -- and name the group they need.
        return self._json(200, {
            "authenticated": True, "sub": s["sub"], "email": s.get("email"),
            "name": s.get("name"), "groups": s.get("groups") or [],
            "may_mint": may_mint, "is_admin": is_admin,
            "required_group": CFG.user_group or None,
            "secure": self._secure()})

    def _health(self) -> None:
        h = STORE.health() if STORE else {}
        h["login_configured"] = CFG.login_configured()
        h["revocation_staleness_seconds"] = REFRESH_SECONDS
        self._json(200, h)

    def _authenticate_key(self, presented: str):
        """Resolve a presented credential to a key row, or None.

        ONE place, used by both the proxy and the stats endpoint. They diverged
        once, when a migration hatch was added to the proxy and not to stats, so
        the same credential could run inference but not read the dashboard. Two
        copies of an auth check is two answers to one question.
        """
        return STORE.authenticate(presented, time.time()) if STORE else None

    def _bearer(self) -> str:
        presented = self.headers.get("Authorization", "")
        return presented[7:].strip() if presented.startswith("Bearer ") else presented

    def _stats(self) -> None:
        """The dashboard's numbers, behind ONE auth authority.

        Accepts either a browser SESSION or an API key, because the two audiences
        are different: a person signs in, a script carries a key. Previously the
        page's only way in was pasting the shared key, which is precisely the
        thing per-user sign-in exists to replace.
        """
        who = self._reader()
        try:
            up = http.client.HTTPConnection("127.0.0.1", CFG.dash_port, timeout=15)
            up.putrequest("GET", "/api/stats", skip_host=True,
                          skip_accept_encoding=True)
            up.putheader("Host", f"127.0.0.1:{CFG.dash_port}")
            if DASHBOARD_KEY:
                up.putheader("Authorization", f"Bearer {DASHBOARD_KEY}")
            up.endheaders()
            r = up.getresponse()
            body = r.read()
            up.close()
        except Exception as exc:
            sys.stderr.write(f"stats proxy failed: {exc}\n")
            return self._err(502, "the stats collector is not answering")
        if who["via"] == "public" and r.status == 200:
            # Parsed and re-serialised rather than relayed, because the public
            # view is an allow-list and cannot be expressed as a passthrough. An
            # unparseable payload becomes a 502 rather than leaking the bytes:
            # failing to project is not a reason to skip projecting.
            try:
                full = json.loads(body)
            except ValueError:
                return self._err(502, "the stats collector answered unreadably")
            return self._json(200, _public.stats(full))
        self.send_response(r.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self._body(body)

    def _servers(self) -> None:
        """Every registered server and what it serves.

        Session OR key, like /api/stats: this says which GPUs hold which models,
        which is what someone needs before choosing a base URL. It is never
        public -- it names internal hosts.
        """
        who = self._reader()

        # This node is described from its own PROBE, not asserted. The page used
        # to draw it as permanently online, which is the one server whose state
        # it could not actually have known.
        def measured(sid):
            """What this node has actually seen from a server, aggregated over
            every model it served -- so the panel reports evidence, not specs."""
            m = THROUGHPUT.get((sid, None)) or {}
            return {"prefill_rate": m.get("prefill_rate"),
                    "mean_service": m.get("mean_service"),
                    "samples": m.get("samples", 0)}

        out = [dict({"id": _ups.LOCAL, "base_url": None, "enabled": True,
                     "note": "this machine", "gpus": None, "problems": [],
                     "needs_key": True, "kind": "local", "priority": 0,
                     "pool_member": True, "owner": None,
                     "state": LOCAL_STATE.get("state") or "unknown",
                     "models": LOCAL_STATE.get("models") or [],
                     # The node's own faults, so the fleet view and the
                     # orphaned-model check see what the node panel already saw.
                     "node_problems": LOCAL_STATE.get("problems") or [],
                     "error": LOCAL_STATE.get("error"),
                     "last_seen": LOCAL_STATE.get("last_seen"),
                     "in_flight": INFLIGHT.get(_ups.LOCAL, 0), "idle_pipes": None,
                     "route": "/u/" + _ups.LOCAL + "/v1"},
                    **measured(_ups.LOCAL))]
        servers = _servers_all()
        view = _state_view(servers)
        try:
            names = STORE.user_names([u.owner for u in servers]) if STORE else {}
        except Exception:
            names = {}
        for u in servers:
            st = view.get(u.id) or {"state": "unknown", "models": [],
                                    "error": None, "last_seen": None}
            row = u.public()
            row["owner_name"] = names.get(u.owner)
            # WHETHER a key is held, never the key. A dialled server with one is
            # not the configuration fault an unauthenticated one is.
            row["needs_key"] = row.get("needs_key") or bool(UP_KEYS.get(u.id))
            row.update(state=st.get("state", "unknown"), models=st.get("models") or [],
                       error=st.get("error"), last_seen=st.get("last_seen"),
                       # The path a client points at, so the page never has to
                       # construct it and get it subtly wrong.
                       route="/u/" + u.id + "/v1",
                       in_flight=(POOL.in_flight(u.id) if u.kind == _ups.KIND_TUNNEL
                                  else INFLIGHT.get(u.id, 0)),
                       # None rather than 0 for a direct server: it has no pipes,
                       # which is a different statement from having none free.
                       idle_pipes=(POOL.idle(u.id) if u.kind == _ups.KIND_TUNNEL
                                   else None),
                       slots=st.get("slots"),
                       agent_version=st.get("agent_version"),
                       cards=st.get("cards"),
                       **measured(u.id))
            out.append(row)
        full = {
            # The interval that ACTUALLY runs, not a constant beside it: the
            # housekeeper's own period is what re-probes these servers, and a
            # panel whose job is honesty about what it knows must not overstate
            # how stale its knowledge can be.
            "servers": out, "poll_seconds": REFRESH_SECONDS,
            "registry_configured": bool(REGISTRY_PATH),
            # The plain path IS the virtual server, so the page can say so
            # rather than hard-coding a convention that might change.
            "default_route": "/v1",
            "auto_route": "/u/" + _ups.AUTO + "/v1",
            "balanced_paths": list(_ups.BALANCED_PATHS),
            "peek_bytes": _peek.PEEK_BYTES,
            "pipe_wait_seconds": PIPE_WAIT_SECONDS,
            # Who is asking, so the page knows which buttons it may offer. It
            # decides nothing: every action re-checks server-side.
            "you": {"sub": who["sub"], "is_admin": who["is_admin"]},
            "routing": dict(ROUTE_WHY)}
        # What is wrong, per server and for the fleet. Computed from the rows
        # just built rather than from a second query, so the panel and the
        # problem list cannot describe different moments.
        # What is holding for a seat, per server. On this build llama.cpp's own
        # queue metrics are unreachable (/metrics 401s), so this is the ONLY
        # queue visibility there is -- and a queue that holds silently is
        # indistinguishable from a node that is simply slow.
        seats = ADMIT.snapshot()
        for row in out:
            row["admission"] = [
                dict(v, model=k.split("/", 1)[1]) for k, v in sorted(seats.items())
                if k.split("/", 1)[0] == row["id"]]
        advertised = set()
        for row in out:
            row["faults"] = (row.pop("node_problems", None) or []) \
                + _health.server_problems(row)
            advertised.update(row.get("models") or [])
        full["problems"] = _health.orphaned_models(advertised, out)
        if who["via"] == "public":
            # Capability and load, with nothing that says where anything lives.
            # Projected by allow-list in publicview.py, so a field added above
            # is absent here until someone names it.
            return self._json(200, _public.servers(full))
        self._json(200, full)

    # --- discovery ----------------------------------------------------------
    def _models(self) -> None:
        """The models a client pointed at the default route can actually reach.

        THE FLEET'S UNION, not this node's list. Until now this answered with
        only local models, which was a lie of a specific and confusing kind: a
        client would be routed to a remote for a model its own model list had
        never offered it, and a model that WAS reachable looked unavailable.

        Only servers that are online AND in the balanced pool are included,
        because those are the ones `/v1` will actually use -- listing an
        unpromoted server's models would produce a 404 for something this
        endpoint had just advertised.

        Deliberately kept PUBLIC and ids-only for remote entries. Clients
        genuinely need discovery before they have a key, and naming which server
        holds what is fleet composition -- that belongs in the authenticated
        /api/servers, which already reports it.
        """
        data = self._fleet_models()
        body = json.dumps({"object": "list", "data": data}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self._body(body)

    def _fleet_models(self) -> list:
        """The union, as a list of model entries.

        Factored out because the chat panel must validate against EXACTLY what
        discovery advertises. Two answers to "what can this node serve" is how a
        panel comes to offer a model the balancer then refuses -- or worse, one
        llama.cpp silently substitutes.
        """
        local, _status = self._local_models_payload()
        data = list(local.get("data") or [])
        seen = {m.get("id") for m in data if isinstance(m, dict)}
        # The dashboard owns the sanitiser, but it does not own the FACT. If it is
        # unreachable, this node's own probe of its runtime still knows what it
        # serves, and discovery losing local models because a telemetry process
        # restarted would be the wrong way round.
        for mid in LOCAL_STATE.get("models") or []:
            if mid and mid not in seen:
                seen.add(mid)
                data.append({"id": mid, "object": "model",
                             "owned_by": "qwen-turing"})
        state = _state_view(_servers_all())
        extra = []
        for u in _servers_all():
            if not (u.usable and u.pool_member):
                continue
            if (state.get(u.id) or {}).get("state") != "online":
                continue
            for mid in (state.get(u.id) or {}).get("models") or []:
                if mid and mid not in seen:
                    seen.add(mid)
                    # The same shape as a local entry, minus anything that would
                    # say where it lives.
                    extra.append({"id": mid, "object": "model",
                                  "owned_by": "qwen-turing"})
        return data + extra

    def _served_model_ids(self) -> set:
        return {m.get("id") for m in self._fleet_models() if isinstance(m, dict)}

    def _local_models_payload(self) -> tuple[dict, int]:
        """This node's list, from the dashboard -- which is where the sanitiser
        lives. Fetched rather than reimplemented: llama.cpp's own list carries
        each child's argv, and two copies of that filter is one too many."""
        try:
            up = http.client.HTTPConnection("127.0.0.1", CFG.dash_port, timeout=10)
            up.putrequest("GET", "/v1/models", skip_host=True,
                          skip_accept_encoding=True)
            up.putheader("Host", f"127.0.0.1:{CFG.dash_port}")
            if DASHBOARD_KEY:
                up.putheader("Authorization", f"Bearer {DASHBOARD_KEY}")
            up.endheaders()
            r = up.getresponse()
            raw = r.read()
            up.close()
            doc = json.loads(raw) if r.status == 200 else {}
            return (doc if isinstance(doc, dict) else {}), r.status
        except Exception as exc:
            sys.stderr.write(f"local model list unavailable: {exc}\n")
            # Degrade to the fleet's remote entries rather than to an error: a
            # client asking what it can reach is better served by a partial answer
            # than by a 502.
            return {"object": "list", "data": []}, 200

    # --- attaching servers -------------------------------------------------
    def _json_body(self) -> dict:
        """The request body as a dict, bounded. Never raises: a malformed body is
        an empty one, and every caller validates what it needs anyway."""
        raw = b""
        while True:
            buf = self._read_body(CHUNK)
            if not buf:
                break
            raw += buf
            if len(raw) > 64 * 1024:
                break
        try:
            out = json.loads(raw or b"{}")
            return out if isinstance(out, dict) else {}
        except Exception:
            return {}

    def _enrol(self) -> None:
        """Issue a single-use token for attaching one server.

        Self-service, like minting a key: the audience that may use this node may
        also add capacity to it. What it does NOT grant is a place in the default
        route -- that needs promotion, because a server declares its own model ids
        and inserting a stranger's hardware into everyone's /v1 is a different act.
        """
        session = self._require_session()
        if not session:
            return
        if not self._secure():
            # The token is a credential in transit. Same rule as minting a key.
            return self._err(400, "attaching a server requires HTTPS",
                             "insecure_transport")
        may, _ = self._may(session)
        if not may:
            return self._err(403, f"your account is not a member of the group "
                                  f"required to attach a server "
                                  f"({CFG.user_group or 'unset'})", "not_authorised")
        body = self._json_body()
        kind = (body.get("kind") or _ups.KIND_TUNNEL).strip()
        server_id = (body.get("server_id") or "").strip()
        try:
            token = STORE.enrol_token(
                session["sub"], server_id, kind=kind,
                base_url=(body.get("base_url") or "").strip() or None,
                note=(body.get("note") or "").strip()[:120] or None,
                gpus=(body.get("gpus") or "").strip()[:120] or None,
                api_key=(body.get("api_key") or "").strip() or None)
        except ValueError as exc:
            return self._err(400, str(exc), "invalid_request")
        host = CFG.public_fqdn or "<this-node>"
        self._json(201, {
            "server_id": server_id, "kind": kind, "token": token,
            "expires_in_seconds": _ks_enrol_ttl(),
            "shown_once": True,
            # The exact command, so nobody has to assemble it from prose.
            "command": (f"qwen-turing-agent enrol --node {host} "
                        f"--token {token} --target http://127.0.0.1:8080")
                       if kind == _ups.KIND_TUNNEL else
                       (f"qwen-turing-agent enrol --node {host} --token {token} "
                        f"--static")})

    def _agent_enrol(self) -> None:
        """Trade a one-time token for a durable server credential.

        No session: the agent is a machine that was handed a token by a person.
        HTTPS is still required -- the credential it receives is long-lived.
        """
        if not self._secure():
            return self._err(400, "enrolment requires HTTPS", "insecure_transport")
        token = (self._json_body().get("token") or "").strip()
        out = STORE.redeem_token(token) if STORE else None
        if out is None:
            # One answer for unknown, expired, already used and forged. The caller
            # is unauthenticated and anything more specific is an oracle.
            return self._err(401, "that enrolment token is not usable",
                             "invalid_token")
        server_id, credential = out
        self._json(201, {"server_id": server_id, "credential": credential,
                         "heartbeat_seconds": HEARTBEAT_SECONDS,
                         "pipes_wanted": DEFAULT_PIPES_WANTED})

    def _server_setting(self, server_id: str, setting: str) -> None:
        """Promotion into the default pool, and the operator tier. Admin only.

        Both are decisions about OTHER people's traffic, which is the same line
        the admin group already draws for revoking other people's keys.
        """
        session = self._require_session()
        if not session:
            return
        _, is_admin = self._may(session)
        if not is_admin:
            group = CFG.admin_group or "the admin group"
            return self._err(403, f"only members of {group} may change this",
                             "not_authorised")
        body = self._json_body()
        try:
            if setting == "pool":
                ok = STORE.set_pool_member(server_id, bool(body.get("pool_member")))
            else:
                ok = STORE.set_priority(server_id, int(body.get("priority") or 0))
        except (ValueError, TypeError) as exc:
            return self._err(400, str(exc), "invalid_request")
        if not ok:
            return self._err(404, "no such server")
        self._json(200, {"server_id": server_id, "updated": setting})

    def _detach(self, server_id: str) -> None:
        """Revoke a server. Its owner, or an admin."""
        session = self._require_session()
        if not session:
            return
        _, is_admin = self._may(session)
        if not STORE.revoke_server(server_id, sub=session["sub"], is_admin=is_admin):
            return self._err(404, "no such server, or not yours to detach")
        # Its credential is dead, so its pipes are no longer trustworthy capacity.
        POOL.drop(server_id)
        with _LOCK:
            AGENT_STATE.pop(server_id, None)
        self._json(200, {"detached": server_id})

    # --- the agent's two connections ---------------------------------------
    def _accept_ws(self):
        """Complete a WebSocket upgrade, or answer and return None.

        Returns the authenticated ServerRow. After this the connection is no
        longer HTTP, so nothing may call send_response on it again.
        """
        if not self._secure():
            # The credential is in a header on this request. Same rule as minting
            # a key: a credential over cleartext is a credential disclosed.
            self._err(400, "an agent connection requires HTTPS",
                      "insecure_transport")
            return None
        row = STORE.authenticate_server(self._bearer()) if STORE else None
        if row is None:
            # Drained and closed like every other refusal: an upgrade request has
            # no body, but the invariant is unconditional because the exception is
            # what gets forgotten.
            self._err(401, "that server credential is not usable",
                      "invalid_server_credential")
            return None
        resp = _ws.handshake_response(self.headers)
        if resp is None:
            self._err(400, "this endpoint speaks WebSocket", "not_an_upgrade")
            return None
        self.close_connection = True
        self.wfile.write(resp)
        self.wfile.flush()
        return row

    def _agent_control(self) -> None:
        """The connection that says a server exists, and keeps saying it.

        Liveness is this connection's existence -- not a probe. A dropped TCP
        connection is a stronger and faster signal than a poll, and a half-open
        one is caught by the heartbeat below.
        """
        row = self._accept_ws()
        if row is None:
            return
        sid = row.server_id
        with _LOCK:
            live = CONTROL.get(sid)
            if live and time.time() - live["seen"] < HEARTBEAT_SECONDS + HEARTBEAT_GRACE:
                # Two boxes fighting over one name would flap the fleet. The
                # loser is told why; a rebooted box gets in once the stale
                # connection is reaped, which its backoff covers.
                self.wfile.write(_ws.encode(_ws.OP_CLOSE, _ws.close_payload(
                    _ws.CLOSE_ALREADY_CONNECTED, "already connected")))
                self.wfile.flush()
                return
            CONTROL[sid] = {"seen": time.time()}
        AGENT_STATE[sid] = {"state": "online", "last_seen": time.time(),
                            "error": None, "gpus": row.gpus, "slots": None}
        reader = _ws.FrameReader(self.rfile)
        last_pong = time.time()
        next_ping = time.time() + HEARTBEAT_SECONDS
        try:
            while True:
                now = time.time()
                if now >= next_ping:
                    self.wfile.write(_ws.encode(_ws.OP_PING, b"hb"))
                    self.wfile.flush()
                    next_ping = now + HEARTBEAT_SECONDS
                if now - last_pong > HEARTBEAT_SECONDS + HEARTBEAT_GRACE:
                    # Half-open: the box lost power rather than closing. Without
                    # this it would look online forever and keep taking traffic.
                    break
                # select rather than a socket timeout, so a timeout never lands
                # in the middle of a frame and desynchronises the reader.
                ready, _w, _x = select.select([self.connection], [], [], 1.0)
                if not ready:
                    continue
                frame = reader.read()
                if frame is None or frame.op == _ws.OP_CLOSE:
                    break
                if frame.op == _ws.OP_PONG:
                    last_pong = time.time()
                elif frame.op == _ws.OP_PING:
                    self.wfile.write(_ws.encode(_ws.OP_PONG, frame.payload))
                    self.wfile.flush()
                elif frame.op == _ws.OP_TEXT:
                    self._agent_said(sid, frame.payload)
                with _LOCK:
                    CONTROL[sid] = {"seen": time.time()}
        except (OSError, ValueError, _ws.ProtocolError) as exc:
            # ValueError included deliberately: a closed buffered writer raises it
            # rather than OSError, and letting it escape would leave a traceback
            # on a worker thread and an attempt to answer 500 on a connection
            # that is no longer HTTP.
            AGENT_STATE.setdefault(sid, {})["error"] = str(exc)[:120]
        finally:
            with _LOCK:
                CONTROL.pop(sid, None)
            AGENT_STATE[sid] = {"state": "offline", "last_seen": time.time(),
                                "error": (AGENT_STATE.get(sid) or {}).get("error"),
                                "gpus": row.gpus, "slots": None}
            # Its pipes are only as good as the agent behind them.
            POOL.drop(sid)
            self.wfile = _Hijacked()

    def _agent_said(self, sid: str, payload: bytes) -> None:
        """Handle a control message. Only what the server says about ITSELF.

        There is deliberately no message here that tells the agent anything about
        where to connect: its target comes from its own config file, so a
        compromised node cannot turn every attached agent into a port scanner
        inside its owner's network.
        """
        try:
            msg = json.loads(payload)
        except Exception:
            return
        if not isinstance(msg, dict):
            return
        state = AGENT_STATE.setdefault(sid, {})
        kind = msg.get("type")
        if kind in ("hello", "capabilities"):
            state.update(state="online", last_seen=time.time(),
                         gpus=(str(msg.get("gpus"))[:120] if msg.get("gpus") else
                               state.get("gpus")),
                         slots=(int(msg["slots"]) if str(msg.get("slots", "")).isdigit()
                                else state.get("slots")),
                         agent_version=str(msg.get("agent_version") or "")[:32])
            if isinstance(msg.get("cards"), list):
                # Per-card telemetry, in the same shape this node's own collector
                # emits, so the panel draws a remote card with the code that
                # already draws a local one. Bounded: this arrives from another
                # machine, and an unbounded list would be a memory hole.
                state["cards"] = [c for c in msg["cards"][:16] if isinstance(c, dict)]
            if STORE:
                try:
                    STORE.touch_server(sid)
                except Exception:
                    pass
        if kind == "hello":
            # Ask what it serves NOW rather than at the next poll. Without this a
            # reconnected server is ineligible for the balanced route until the
            # timer comes round -- it is online, with an empty model list, which
            # reads as "serves nothing".
            #
            # HELLO ONLY. Capabilities arrive every 20 s for as long as the agent
            # is attached, and probing on each of those would spawn a thread and
            # take a pipe three times a minute, forever, to re-answer a question
            # whose answer had not changed.
            threading.Thread(target=_probe_when_ready, args=(sid,),
                             daemon=True).start()

    def _agent_pipe(self) -> None:
        """One idle pipe, offered to the pool.

        The handler thread then parks: returning from it would let the HTTP server
        close the socket the gateway is about to use.
        """
        row = self._accept_ws()
        if row is None:
            return
        pipe = _tunnel.Pipe(self.rfile, self.wfile, sock=self.connection)
        POOL.offer(row.server_id, pipe)
        try:
            pipe.wait_closed()
        except Exception:
            pass
        finally:
            pipe.close()
            self.wfile = _Hijacked()

    # --- keys --------------------------------------------------------------
    def _viewer(self):
        """Who is asking, from a session OR an API key, or None having answered.

        One notion of "authenticated reader" for every read endpoint. Previously
        /api/stats took either while /api/usage demanded a session, so a key
        could read the node's numbers but not its own usage -- an arbitrary line.
        Minting still requires a session, because that is a browser action.
        """
        s = self._session()
        if s:
            _, is_admin = self._may(s)
            return {"sub": s["sub"], "is_admin": is_admin, "via": "session"}
        row = self._authenticate_key(self._bearer())
        if row:
            # A key carries no group claims, so it reads as its owner and never
            # as an admin.
            return {"sub": row.sub, "is_admin": False, "via": "key"}
        self._drain_body()
        self.close_connection = True
        self._json(401, {"error": {"message": "sign in, or present an API key",
                                   "type": "not_authenticated", "code": 401},
                         "login_configured": CFG.login_configured()})
        return None

    def _reader(self) -> dict:
        """Who is reading, WITHOUT refusing a stranger.

        Anyone may read this node -- what it serves, how busy it is, how fast,
        and who is on the leaderboard. `via` says which audience answered, and
        every read handler uses it to choose between the full payload and the
        allow-listed public projection in publicview.py.

        The sibling `_viewer()` still exists and still refuses: it is for reads
        that stay private. This one never writes a response, so a caller can rely
        on getting an identity back.
        """
        s = self._session()
        if s:
            _, is_admin = self._may(s)
            return {"sub": s["sub"], "is_admin": is_admin, "via": "session"}
        row = self._authenticate_key(self._bearer())
        if row:
            return {"sub": row.sub, "is_admin": False, "via": "key"}
        return {"sub": None, "is_admin": False, "via": "public"}

    def _require_session(self):
        s = self._session()
        if not s:
            self._err(401, "not logged in", "not_authenticated")
            return None
        return s

    def _list_keys(self) -> None:
        s = self._require_session()
        if not s:
            return
        _, is_admin = self._may(s)
        want_all = "all=1" in (self.path.split("?", 1)[1] if "?" in self.path else "")
        rows = STORE.list_keys(s["sub"], all_users=bool(want_all and is_admin))
        self._json(200, {"keys": [r.as_public() for r in rows],
                         "all_users": bool(want_all and is_admin)})

    def _mint(self) -> None:
        s = self._require_session()
        if not s:
            return
        if not self._secure():
            # A key minted over cleartext is a key already disclosed.
            return self._err(400, "key creation requires HTTPS", "insecure_transport")
        may_mint, _ = self._may(s)
        if not may_mint:
            return self._err(403,
                             f"your account is not a member of the group required "
                             f"to create keys ({CFG.user_group or 'unset'})",
                             "not_authorised")
        try:
            raw = b""
            while True:
                buf = self._read_body(CHUNK)
                if not buf:
                    break
                raw += buf
                if len(raw) > 64 * 1024:      # a key request is tiny
                    break
            payload = json.loads(raw or b"{}")
        except Exception:
            payload = {}
        label = (payload.get("label") or "").strip()[:64] or None
        ttl = payload.get("ttl_days")
        ttl = int(ttl) if isinstance(ttl, (int, float)) and 0 < ttl <= 3650 else None

        full, row = STORE.mint(s["sub"], label=label, ttl_days=ttl)
        # The ONLY time the secret leaves this process. Said plainly to the page.
        self._json(201, {"key": full, "shown_once": True, **row.as_public()})

    def _revoke(self, key_id: str) -> None:
        s = self._require_session()
        if not s:
            return
        if not self._secure():
            return self._err(400, "revocation requires HTTPS", "insecure_transport")
        _, is_admin = self._may(s)
        ok = STORE.revoke(key_id, sub=s["sub"], is_admin=is_admin)
        if not ok:
            return self._err(404, "no such key, or not yours to revoke")
        # Applied locally before this returns, so the caller can rely on it.
        self._json(200, {"revoked": key_id, "effective": "immediately on this node",
                         "other_nodes_within_seconds": REFRESH_SECONDS})

    def _usage_api(self) -> None:
        """The scoreboard, and the per-key detail behind it.

        Both span every user. This is a shared node and the point of a
        leaderboard is comparison -- one that showed only your own row would not
        be one. It stays behind authentication: a lab scoreboard, never a public
        one, and the public payload allow-list cannot grow a field from here.

        `mine=1` narrows the detail table to the caller, for someone who only
        wants to audit their own keys.
        """
        who = self._reader()
        if who["via"] == "public":
            # The scoreboard, by display name. No email, no sub -- and no key
            # table: that names each person's keys, which is the one part of this
            # payload that is about individuals rather than about the node.
            return self._json(200, {
                "leaderboard": _public.leaderboard(STORE.leaderboard()),
                "usage": [], "scope": "public", "you": None,
                "is_admin": False, "public": True})
        query = self.path.split("?", 1)[1] if "?" in self.path else ""
        mine = "mine=1" in query
        self._json(200, {
            "leaderboard": STORE.leaderboard(),
            "usage": STORE.usage(sub=who["sub"] if mine else None),
            "scope": "mine" if mine else "everyone",
            "you": who["sub"],
            "is_admin": who["is_admin"]})

    def _client_gone(self) -> bool:
        """Has the caller hung up? Checked WITHOUT consuming anything.

        MSG_PEEK rather than a read: at admission time the request body has not
        been forwarded yet, so consuming a byte here would corrupt it. Readable
        with zero bytes waiting is the peer having closed.

        Matters because the queue is FIFO: a caller that walked away would
        otherwise hold the head of the line until its timeout, and everyone
        behind it waits on a ghost.
        """
        try:
            import select
            import socket as _socket
            sock = self.connection
            ready, _, _ = select.select([sock], [], [], 0)
            if not ready:
                return False
            return sock.recv(1, _socket.MSG_PEEK) == b""
        except Exception:
            # Unknown is not gone. Guessing "gone" would cancel a live request.
            return False

    # --- the chat panel ----------------------------------------------------
    def _chat(self) -> None:
        """Inference bought with a SESSION, for the dashboard's chat panel.

        The one endpoint here where a cookie pays for GPU time, which is why the
        same-origin check comes first -- before the body is read, before a model
        is resolved, before anything is routed. A cookie travels automatically,
        so without that check any page on the internet could spend this node.

        Everything after validation is the ORDINARY balanced path: the request is
        rewritten into the chat-completions request it actually is and handed to
        the same proxy every key-holder uses. Eligibility, scheduling, warm-cache
        affinity, streaming and accounting are therefore identical -- a second
        implementation of routing for the panel is how the panel would come to
        disagree with the balancer.
        """
        session = self._require_session()
        if not session:
            return
        if not self._secure():
            return self._err(403, "the chat panel needs HTTPS",
                             "insecure_transport")
        if not _chat.same_origin(self.headers.get, self.headers.get("Host") or ""):
            return self._err(403, "this endpoint answers only this node's page",
                             "cross_origin")

        raw = b""
        while len(raw) <= _chat.MAX_CHARS * 4:
            buf = self._read_body(CHUNK)
            if not buf:
                break
            raw += buf
        self._drain_body()

        body, why = _chat.validate(raw, self._served_model_ids())
        if body is None:
            return self._err(400, why, "invalid_request")

        blob = json.dumps(body).encode()
        # The rewritten body replaces the peek, and the request length must be
        # corrected with it: _send_upstream_request copies the client's headers
        # verbatim, so a stale Content-Length would truncate the prompt upstream.
        self._peeked = blob
        self._body_left = 0
        if self.headers.get("Content-Length") is None:
            self.headers["Content-Length"] = str(len(blob))
        else:
            self.headers.replace_header("Content-Length", str(len(blob)))
        # The body this now sends IS JSON, whatever the panel labelled its own
        # request. The peek that finds the model checks this header before
        # scanning, so a mislabelled request would route blind.
        if self.headers.get("Content-Type") is None:
            self.headers["Content-Type"] = "application/json"
        else:
            self.headers.replace_header("Content-Type", "application/json")
        # Balanced like any other completion. The path IS what the machinery keys
        # on -- BALANCED_PATHS, the model peek, the affinity -- so it is set here
        # rather than special-cased in five places.
        self.path = "/v1/chat/completions"
        self._proxy("POST", identity=(_chat.USAGE_KEY_ID, session["sub"]))

    # --- planning: where does this request go? ------------------------------
    @staticmethod
    def _local_online() -> bool:
        """Is the local runtime answering?

        False ONLY when a probe ran and failed. `unknown` -- no probe has
        completed yet -- counts as available: refusing every request because a
        liveness probe is late would be failing closed on telemetry, which is
        the wrong way round for anything that is not authentication.
        """
        return (LOCAL_STATE.get("state") or "unknown") != "offline"

    def _last_server(self) -> str | None:
        """The server this key used last, from memory or from recorded usage.

        Read through to the store on a miss so a gateway restart does not
        scatter everybody onto cold slots -- the affinity is worth ~10x on
        prompt processing and it must survive a deploy.
        """
        kid = self._auth_key_id or ""
        last = LAST_SERVER.get(kid)
        if last is None and kid and STORE:
            try:
                last = STORE.last_upstream(kid, _ups.BALANCED_PATHS)
            except Exception:
                last = None
            if last:
                LAST_SERVER[kid] = last
        return last

    def _peek_body(self) -> tuple[str | None, bool]:
        """The model this request asks for, from a bounded prefix of the body.

        Returns (model, conclusive). Every "I cannot tell" case answers None,
        and the caller treats that as "keep it here" -- never as "any server
        will do", because llama.cpp would answer with the wrong model rather
        than refuse.
        """
        if self._body_left <= 0 and not self._peeked:
            # Nothing to look at. A chunked body reaches here with no length and
            # nothing buffered, and is left alone rather than reframed, so the
            # existing pass-through is unchanged.
            #
            # The `not self._peeked` half matters: the chat panel rewrites the
            # request and hands over a body that is ALREADY buffered with nothing
            # left to read. Testing the length alone declared that request
            # unreadable and kept every chat turn on this node -- a model only
            # another server had was served locally, which is the substitution
            # this whole routing layer exists to prevent.
            return None, True
        ctype = (self.headers.get("Content-Type") or "").lower()
        if "json" not in ctype:
            return None, True
        if self.headers.get("Content-Encoding"):
            return None, True          # compressed; not worth inflating here
        while len(self._peeked) < _peek.PEEK_BYTES and self._body_left > 0:
            want = min(CHUNK, _peek.PEEK_BYTES - len(self._peeked))
            buf = self._read_body(want)
            if not buf:
                break
            self._peeked += buf
        return _peek.peek_model(self._peeked)

    def _load(self, servers) -> dict:
        """Requests already in flight per server, this node included.

        Counted here rather than asked of each server, because this process is
        the only thing that knows about every request it has sent -- and a remote
        that reports its own queue is reporting work from other callers too,
        which is why a tunnelled server's pipe accounting is used where it exists.
        """
        out = {_ups.LOCAL: INFLIGHT.get(_ups.LOCAL, 0)}
        for u in servers:
            if u.kind == _ups.KIND_TUNNEL:
                out[u.id] = POOL.in_flight(u.id)
            else:
                out[u.id] = INFLIGHT.get(u.id, 0)
        return out

    def _ready(self, servers) -> dict:
        """Can each server start work right now?

        Only a tunnelled server can be genuinely unable to: its capacity is the
        pipes it has offered. Not an exclusion -- the scheduler ranks it last, so
        the balanced route goes around a busy box while a pin can still reach it.
        """
        out = {_ups.LOCAL: True}
        for u in servers:
            out[u.id] = POOL.ready(u.id) if u.kind == _ups.KIND_TUNNEL else True
        return out

    def _prompt_tokens(self) -> float:
        """A rough token count for this request, for the prefill estimate.

        From Content-Length, at ~4 bytes per token. Deliberately crude: it is
        already known, it costs nothing, and the scheduler only needs to tell a
        500-token request from a 100k one. Counting properly would mean
        tokenising the body, which is the whole thing this gateway refuses to do.
        """
        n = int(self.headers.get("Content-Length") or 0)
        return n / 4.0 if n else 0.0

    def _plan(self, remote: bool):
        """Where this request goes: an Upstream, _ups.LOCAL, or ANSWERED.

        ANSWERED means a refusal has already been written -- and every one of
        those paths drains the body first, because a reply that leaves an unread
        body in the socket corrupts the NEXT request on the connection.
        """
        raw = self.path.split("?", 1)[0]
        if not remote:
            # No server named. THIS IS THE DEFAULT ROUTE, and the default is the
            # virtual server: a client configured against /v1 is balanced
            # without being reconfigured.
            self._route_path = raw
            return self._balance(raw)

        if raw == "/u/" + _ups.AUTO or raw.startswith("/u/" + _ups.AUTO + "/"):
            self._route_path = self._strip_prefix(raw)
            return self._balance(self._route_path)

        # The MERGED list: a pinned name may be file-configured or attached, and
        # consulting only the file made every attached server unreachable by pin.
        resolved = _ups.route(raw, _servers_all())
        if resolved is None:
            self._err(503, "that server is not available on this node",
                      "upstream_unavailable")
            return ANSWERED
        pinned, self._route_path = resolved
        if pinned == _ups.LOCAL:
            if not self._local_online():
                self._err(503, "this node's runtime is not answering",
                          "upstream_offline")
                return ANSWERED
            self._decided(_ups.LOCAL, "pinned")
            return _ups.LOCAL
        st = _state_view([pinned]).get(pinned.id) or {}
        if st.get("state") == "offline":
            seen = (f" (last seen {int(time.time() - st['last_seen'])}s ago)"
                    if st.get("last_seen") else " and has not been seen")
            self._err(503, f"{pinned.id} is not answering" + seen,
                      "upstream_offline")
            return ANSWERED
        why = self._pin_serves_it(pinned.id, st.get("models"))
        if why is None:
            return ANSWERED
        self._decided(pinned.id, why)
        return pinned

    def _pin_serves_it(self, server: str, models) -> str | None:
        """Can the server the caller NAMED serve the model they named?

        A pin says which machine, not which model -- and llama.cpp's silent
        substitution does not care how the destination was chosen. Pinning
        `/u/box/v1` and asking for a model that box has not got would return
        someone else's weights with a 200, which is the exact failure this whole
        routing change exists to prevent. So the same eligibility check applies,
        with an explicit escape: `X-Route-Force: 1` sends it anyway.

        Only checked when the server's own list is KNOWN and the endpoint is one
        that names a model. An unknown list defers to the server.

        Returns the reason to record, or None having already answered.
        """
        if not _ups.balanceable(self._route_path or ""):
            return "pinned"
        if not models:
            return "pinned"
        if self.headers.get("X-Route-Force", "").strip() in ("1", "true", "yes"):
            return "forced"
        model, _ = self._peek_body()
        if model is None or model in models:
            return "pinned"
        self._decided(server, "refused")
        self._err(404, f"{server} does not serve {model} -- it would answer with "
                       f"a different model rather than refuse. Send it anyway "
                       f"with X-Route-Force: 1, or use the default /v1.",
                  "model_not_found")
        return None

    def _decided(self, server: str, why: str) -> None:
        """Record a routing decision, for the response headers and the counters.

        Counted because a balancer that quietly stops balancing looks identical
        from outside to one that is working: if the peek starts failing, every
        request goes local and the only visible trace is this tally.
        """
        self._auto_choice, self._route_why = server, why
        ROUTE_WHY[why] = ROUTE_WHY.get(why, 0) + 1

    def _balance(self, path: str):
        """Choose a server for a request that named none."""
        if not _ups.balanceable(path):
            # /health, /props, /metrics, /slots ... describe THIS machine.
            # Answering them from another one would report someone else's GPUs.
            self._decided(_ups.LOCAL, "not-balanced")
            return _ups.LOCAL

        model, conclusive = self._peek_body()
        # Kept for admission control, which needs the tier the caller named.
        self._req_model = model
        servers = _servers_all()
        state = _state_view(servers)
        chosen, why, est = _ups.pick_auto(
            servers, state, self._last_server(), model=model,
            local_online=self._local_online(),
            local_models=LOCAL_STATE.get("models"),
            stats=THROUGHPUT, load=self._load(servers), ready=self._ready(servers),
            prompt_tokens=self._prompt_tokens())
        self._route_est = est
        if why == "blind" and conclusive:
            # Three different problems, three different labels. `unnamed` is the
            # client's choice, `blind` is a peek budget too small, and `unframed`
            # is a body with no Content-Length -- which is never scanned, so it
            # would otherwise hide inside `unnamed` and look like client
            # behaviour instead of a framing case.
            why = ("unframed" if self._body_left <= 0
                   and self.headers.get("Transfer-Encoding") else "unnamed")

        if chosen is None:
            self._decided("none", "refused")
            servers_for = servers
            if model is None:
                self._err(503, "cannot tell which model this request asks for, "
                               "and this node's runtime is not answering",
                          "no_upstream")
                return ANSWERED
            where = _ups.servers_for(servers_for, state, model)
            # Three different situations that a single message would blur:
            #   * a server has it and is answering, but is not in the balanced
            #     pool -- the caller can reach it directly, so SAY so;
            #   * a server has it and is not answering -- waiting might help;
            #   * nobody has it -- waiting will not.
            by_id = {u.id: u for u in servers_for}
            unpromoted = [s for s in where
                          if (state.get(s) or {}).get("state") == "online"
                          and by_id.get(s) is not None
                          and not by_id[s].pool_member]
            offline = [s for s in where if s not in unpromoted]
            if unpromoted:
                # The pin path is told only to the servers' OWNER. Handing a
                # stranger the address of somebody's unvetted machine is exactly
                # what admin-gated promotion exists to prevent -- they would be
                # sending their prompts to hardware nobody has vetted.
                mine = [s for s in unpromoted
                        if by_id.get(s) is not None
                        and by_id[s].owner
                        and by_id[s].owner == getattr(self, "_auth_sub", None)]
                if mine:
                    pins = ", ".join(f"/u/{s}/v1" for s in mine)
                    self._err(404, f"{model} is served by {', '.join(mine)}, "
                                   f"which is not in the balanced pool -- your "
                                   f"own server, so reach it directly at {pins}, "
                                   f"or ask an administrator to add it",
                              "model_not_in_pool")
                else:
                    self._err(404, f"no server in the balanced pool serves "
                                   f"{model}", "model_not_in_pool")
            elif offline:
                self._err(503, f"{model} is served by "
                               f"{', '.join(offline)}, which is not answering",
                          "upstream_offline")
            else:
                # 404, not 503: nothing here serves it, so waiting will not help.
                self._err(404, f"no server on this node serves {model}",
                          "model_not_found")
            return ANSWERED

        self._decided(chosen, why)
        if chosen == _ups.LOCAL:
            return _ups.LOCAL
        for u in servers:
            if u.id == chosen:
                return u
        self._err(503, "no server is available right now", "no_upstream")
        return ANSWERED

    @staticmethod
    def _strip_prefix(path: str) -> str:
        """Drop the leading /u/<server> from a routed path, by SEGMENT COUNT
        rather than by name -- the name may be a real server or the virtual one,
        and both must strip identically."""
        return "/" + path.split("/", 3)[3] if path.count("/") >= 3 else "/"

    def _refuse_key(self) -> None:
        """One answer for every authentication failure -- unknown key, wrong
        secret, revoked, expired. Distinguishing them to an unauthenticated
        caller is an oracle. WWW-Authenticate so client libraries prompt."""
        self._drain_body()
        self.close_connection = True
        body = json.dumps({"error": {
            "message": "invalid or revoked API key", "type": "invalid_api_key",
            "code": 401}}).encode()
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Bearer realm="qwen-turing"')
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self._body(body)

    # --- the proxy ---------------------------------------------------------
    def _proxy(self, method: str, remote: bool = False,
               identity: tuple | None = None) -> None:
        """Relay an inference request. `identity` overrides key authentication.

        Only the chat panel passes one, having already established WHO from a
        verified session. It carries a sentinel key id rather than a minted key,
        so usage attributes to the person without creating a real credential they
        can neither see nor revoke.
        """
        if identity is None:
            row = self._authenticate_key(self._bearer())
            if row is None:
                return self._refuse_key()
            key_id, sub = row.key_id, row.sub
        else:
            key_id, sub = identity

        self._auth_key_id = key_id
        # Whose request this is, so a refusal can tell an owner about their own
        # server without telling everyone else about it.
        self._auth_sub = sub
        self._route_path = None
        plan = self._plan(remote)
        if plan is ANSWERED:
            return                         # _plan already answered
        # None == the local runtime, which is what both "pinned here" and
        # "balanced here" come to.
        target = None if plan == _ups.LOCAL else plan
        # Remember where this key went, so the next balanced request goes back
        # to the same warm prefix cache.
        #
        # NOT for requests that were never balanced. A client polling /v1/models
        # or /health would otherwise keep resetting its own affinity to this
        # node, and its next completion would abandon a warm remote slot for a
        # cold local one -- the exact loss the affinity exists to prevent.
        if self._route_why != "not-balanced":
            LAST_SERVER[key_id] = target.id if target else _ups.LOCAL

        started = time.time()
        acc = _usage.StreamAccountant()
        status = 0
        streamed = False
        where = target.id if target else _ups.LOCAL

        # --- admission: HOLD for a seat rather than over-subscribe ----------
        # A context tier on this node is a contract nothing enforced. Two
        # sessions could claim more of one model's KV pool than exists, and
        # llama.cpp does not refuse -- it dies, taking every live session with
        # it. So a request that does not fit waits here, before it can take a
        # tunnel pipe or reach a runtime.
        lease = pool = None
        try:
            lease, pool, refusal = self._admit(where)
        except NoCapacity as exc:
            self._decided(str(exc), "too-large")
            return self._err(413, str(exc), "context_too_large")
        if refusal:
            self._decided(where, "full")
            return self._err(503, refusal, "no_capacity")

        with _LOCK:
            INFLIGHT[where] = INFLIGHT.get(where, 0) + 1
        try:
            status, streamed = self._relay(method, acc, target)
        except NoCapacity as exc:
            # Not an upstream error: the server is fine and simply busy. Answering
            # 502 would send people looking at the wrong machine.
            self._decided(str(exc), "no-capacity")
            try:
                self._err(503, f"{exc} has no free capacity right now",
                          "no_capacity")
            except Exception:
                pass
            status = 503
        except (BrokenPipeError, ConnectionResetError):
            # The client went away mid-stream. The terminal chunk never arrived,
            # so the counts are UNKNOWN -- recorded as truncated, never zero.
            status = 499
        except Exception as exc:
            sys.stderr.write(f"proxy error: {exc}\n")
            try:
                self._err(502, "upstream error")
            except Exception:
                pass
            status = 502
        finally:
            # Released on EVERY path. A leaked lease shrinks the pool
            # permanently, and that failure looks like a node that mysteriously
            # got slower rather than like a bug.
            if pool is not None:
                pool.release(lease)
            with _LOCK:
                INFLIGHT[where] = max(0, INFLIGHT.get(where, 1) - 1)
            u = acc.result()
            try:
                STORE.record_usage({
                    "ts": started, "key_id": key_id, "sub": sub,
                    "model": u.model,
                    "upstream": target.id if target else "local",
                    "endpoint": self.path.split("?", 1)[0],
                    "prompt_tokens": u.prompt_tokens,
                    "completion_tokens": u.completion_tokens,
                    "cached_tokens": u.cached_tokens,
                    "prompt_ms": u.prompt_ms, "predicted_ms": u.predicted_ms,
                    "status_code": status, "streamed": streamed,
                    "truncated": u.truncated})
            except Exception as exc:
                # Accounting must never break serving.
                sys.stderr.write(f"usage not recorded: {exc}\n")

    def _admit(self, where: str):
        """Reserve KV for this request. Returns (lease, pool, refusal_message).

        Only inference is weighed. A model list or a health probe reserves
        nothing: they cost no KV, and making them queue behind a 100k prefill
        would make the node look dead while it was merely busy.

        An unbudgeted request -- a model whose pool size this node does not know
        -- is admitted rather than blocked. Failing closed on missing telemetry
        would turn one unreadable catalog into a total outage, and the tiers this
        protects are the ones we DO know about.
        """
        model = self._req_model
        path = (self._route_path or self.path).split("?", 1)[0]
        if not model or not _ups.balanceable(path):
            return None, None, None
        sizes = POOL_SIZES.get(model)
        if not sizes:
            return None, None, None
        context, slots = sizes
        want = _adm.tier_tokens(model, context, slots)
        if want <= 0:
            return None, None, None
        pool = ADMIT.pool(where, model, context)
        try:
            lease = pool.acquire(want, ADMIT_WAIT_SECONDS,
                                 cancelled=self._client_gone)
        except _adm.TooLarge as exc:
            raise NoCapacity(str(exc))
        if lease is None:
            return None, None, (
                f"{where} is full: {want} tokens of {model}'s "
                f"{context}-token pool were not free within "
                f"{ADMIT_WAIT_SECONDS}s")
        self._admit_waited = lease.waited
        return lease, pool, None

    def _relay(self, method: str, acc, target=None) -> tuple[int, bool]:
        query = self.path.split("?", 1)
        suffix = ("?" + query[1]) if len(query) == 2 else ""
        scheme, host, port = "http", CFG.upstream_host, CFG.upstream_port
        if target is not None and target.kind == _ups.KIND_TUNNEL:
            # The agent forwards to its own configured target, so the path must
            # be what that target expects -- which is what _route_path already is.
            path = (self._route_path or query[0]) + suffix
            key = UP_KEYS.get(target.id, "")
        elif target is None:
            # _route_path is set whenever the request arrived under /u/, which
            # includes auto choosing THIS node.
            path = (self._route_path or query[0]) + suffix
            key = INTERNAL_KEY
        else:
            scheme, host, port, base = _ups.target(
                target, self._route_path or self._strip_prefix(query[0]))
            path = base + suffix
            key = UP_KEYS.get(target.id, "")
        if target is not None and target.kind == _ups.KIND_TUNNEL:
            # No address to dial: this server's capacity arrives as pipes it
            # holds open. Everything below is unchanged -- http.client is
            # speaking over one of those instead of over a socket it opened.
            pipe = POOL.take(target.id, PIPE_WAIT_SECONDS)
            if pipe is None:
                raise NoCapacity(target.id)
            up = http.client.HTTPConnection(target.id, 80, timeout=900)
            up.sock = pipe
        else:
            cls = (http.client.HTTPSConnection if scheme == "https"
                   else http.client.HTTPConnection)
            up = cls(host, port, timeout=900)
        try:
            self._send_upstream_request(up, method, path, host, port, key)
            r = up.getresponse()
            # A model list is the one response this proxy rewrites, and the only
            # one it may: it is small, bounded, and carries a disclosure this node
            # already strips from its own. Everything else streams untouched.
            if path.split("?", 1)[0].endswith("/v1/models") and r.status == 200:
                raw = r.read(MODEL_LIST_MAX)
                clean = self._sanitised_model_list(raw)
                out = clean if clean is not None else raw
                self.send_response(200)
                chosen = getattr(self, "_auto_choice", None)
                if chosen:
                    self.send_header("X-Routed-To", chosen)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(out)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self._body(out)
                return r.status, False
            chunked = self._mirror_response_headers(r)
            self._pump(r, acc, chunked)
            # "streamed" means the CLIENT asked for a stream, which the response
            # content type reports. Chunked transfer encoding is not the same
            # question -- llama.cpp answers a plain JSON completion chunked too,
            # so inferring it from framing marked every request as streamed.
            ctype = (r.getheader("Content-Type") or "").lower()
            return r.status, "event-stream" in ctype
        finally:
            up.close()

    def _send_upstream_request(self, up, method: str, path: str | None = None,
                               host: str | None = None, port: int | None = None,
                               key: str | None = None) -> None:
        path = self.path if path is None else path
        host = CFG.upstream_host if host is None else host
        port = CFG.upstream_port if port is None else port
        key = INTERNAL_KEY if key is None else key
        up.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
        for k, v in self.headers.items():
            if k.lower() in ("connection", "keep-alive", "transfer-encoding",
                             "authorization", "cookie", "host"):
                continue
            up.putheader(k, v)
        up.putheader("Host", f"{host}:{port}")
        # The client's credential is NEVER forwarded. Each backend has its own,
        # so a key that works here grants nothing on the machine behind it.
        if key:
            up.putheader("Authorization", f"Bearer {key}")
        up.endheaders()

        # Whatever was read to find the model goes first, unchanged. The body
        # the upstream sees is byte-identical to the body the client sent.
        if self._peeked:
            up.send(self._peeked)
        # Streamed up in bounded chunks: a 100k prompt is ~400 KB and must not
        # be held in memory just to be forwarded.
        while True:
            buf = self._read_body(CHUNK)
            if not buf:
                break
            up.send(buf)

    def _mirror_response_headers(self, r) -> bool:
        """Copy the upstream's headers through. Returns True when the body must
        be chunked, i.e. the upstream gave no Content-Length (every streamed
        completion)."""
        self.send_response(r.status)
        # Which server actually served this, and why it was chosen. Without
        # these the default route is a black box: nobody can explain their own
        # latency, and "auto quietly became local-always" is invisible.
        chosen = getattr(self, "_auto_choice", None)
        if chosen:
            self.send_header("X-Routed-To", chosen)
        why = getattr(self, "_route_why", None)
        if why:
            self.send_header("X-Routed-Why", why)
        # How long this request HELD for a seat. Reported so a caller can tell a
        # full node from a slow one -- those call for different actions, and
        # without this the wait is indistinguishable from prefill.
        waited = getattr(self, "_admit_waited", 0.0)
        if waited and waited > 0.05:
            self.send_header("X-Queued-Seconds", f"{waited:.2f}")
        est = getattr(self, "_route_est", None)
        if est is not None:
            # What the scheduler PREDICTED, so its arithmetic can be checked from
            # outside against what actually happened.
            self.send_header("X-Routed-Est", f"{est:.1f}")
        length = None
        for k, v in r.getheaders():
            lk = k.lower()
            if lk in ("connection", "keep-alive", "transfer-encoding"):
                continue
            if lk == "content-length":
                length = v
            self.send_header(k, v)
        if length is None:
            self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        return length is None

    _MODEL_FIELDS = ("id", "aliases", "object", "owned_by", "created")

    def _sanitised_model_list(self, body: bytes) -> bytes | None:
        """A remote's own /v1/models, stripped to the fields a client needs.

        A pinned `/u/<id>/v1/models` relays that server's answer -- and if it runs
        llama.cpp, that answer carries each child's argv including the path to its
        API key file. This node has always cleaned its OWN list; relaying someone
        else's raw was the same disclosure with an extra hop. Returns None when
        the body is not a model list, in which case it passes through untouched.
        """
        try:
            doc = json.loads(body)
        except Exception:
            return None
        if not isinstance(doc, dict) or not isinstance(doc.get("data"), list):
            return None
        out = [{k: m[k] for k in self._MODEL_FIELDS if k in m}
               for m in doc["data"] if isinstance(m, dict)]
        return json.dumps({"object": "list", "data": out}).encode()

    def _pump(self, r, acc, chunked: bool) -> None:
        """Relay the body downstream, flushing each chunk as it arrives -- that is
        what makes a completion appear token by token rather than all at once."""
        while True:
            buf = r.read1(CHUNK)
            if not buf:
                break
            acc.feed(buf)
            if chunked:
                self.wfile.write(b"%x\r\n" % len(buf) + buf + b"\r\n")
            else:
                self.wfile.write(buf)
            self.wfile.flush()
        if chunked:
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()

    def finish(self):
        """The framework's epilogue, made harmless.

        A hijacked connection is closed by whoever took it over, so flushing and
        closing it again is expected to fail. Swallowed here rather than in five
        places, because every one of those failures is a worker thread printing a
        traceback about a socket that did its job.
        """
        try:
            super().finish()
        except (OSError, ValueError):
            pass

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))


def _load_credentials(args) -> str | None:
    """Read the three key files. Returns an error message, or None on success.

    All three are read FIRST LINE ONLY: a key file may legitimately hold several
    keys, and read().strip() would join them into a value matching nothing.
    """
    global INTERNAL_KEY, DASHBOARD_KEY
    try:
        INTERNAL_KEY = (open(args.internal_key_file).readline() or "").strip()
    except OSError as exc:
        return f"cannot read the internal key: {exc}"
    if not INTERNAL_KEY:
        return "the internal key file is empty"

    if args.dashboard_key_file:
        try:
            DASHBOARD_KEY = (open(args.dashboard_key_file).readline() or "").strip()
        except OSError as exc:
            return f"cannot read the dashboard key: {exc}"

    return None


def _build_dsn(password_file: str | None) -> str | None:
    """The registry DSN, or None to run mirror-only. Mirror-only is a supported
    mode, not a failure: keys still authenticate and usage still records."""
    host = os.environ.get("QT_DB_HOST")
    if not (host and password_file):
        return None
    pw = (open(password_file).readline() or "").strip()
    return (f"host={host} port={os.environ.get('QT_DB_PORT', '6432')} "
            f"dbname={os.environ.get('QT_DB_NAME', '')} "
            f"user={os.environ.get('QT_DB_USER', '')} password={pw} "
            f"connect_timeout=3 application_name=qwen-turing-gateway")


def main() -> int:
    global STORE
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8082)
    ap.add_argument("--mirror", required=True, help="SQLite mirror path")
    ap.add_argument("--internal-key-file", required=True)
    ap.add_argument("--dashboard-key-file",
                    help="the dashboard's own key, so the gateway can front its "
                         "stats behind one auth authority")
    ap.add_argument("--db-password-file")
    ap.add_argument("--upstreams",
                    help="registry of servers this node can route to")
    args = ap.parse_args()

    problem = _load_credentials(args)
    if problem:
        print(problem, file=sys.stderr)
        return 78

    try:
        dsn = _build_dsn(args.db_password_file)
    except OSError as exc:
        print(f"cannot read the registry password: {exc}", file=sys.stderr)
        return 78

    global REGISTRY_PATH
    REGISTRY_PATH = args.upstreams or ""
    _load_registry()
    _poll_upstreams()
    _poll_local()

    STORE = KeyStore(args.mirror, dsn=dsn)
    STORE.migrate_local()
    if dsn:
        STORE.refresh()
    # AFTER the store exists, not before: the first version of this line sat
    # above it and silently did nothing, so every restart began with an
    # unmeasured fleet and ranked it as if nothing had ever been served.
    _refresh_throughput()

    threading.Thread(target=_housekeeper, daemon=True).start()

    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"gateway on http://{args.host}:{args.port} -> "
          f"{CFG.upstream_host}:{CFG.upstream_port} "
          f"(registry {'configured' if dsn else 'absent'}"
          f"{', stats fronted' if DASHBOARD_KEY else ''}"
          f", {len([u for u in UPSTREAMS if u.usable])} routable server(s))",
          flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
