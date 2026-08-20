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
import oidc as _oidc            # noqa: E402
import upstreams as _ups        # noqa: E402
import usage as _usage          # noqa: E402
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
         "/api/stats", "/api/servers")
UPSTREAM_POLL_SECONDS = 60

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
    for u in found:
        if u.key_file and u.id not in UP_KEYS:
            try:
                UP_KEYS[u.id] = (open(u.key_file).readline() or "").strip()
            except OSError as exc:
                u.problems.append(f"key_file unreadable: {exc.strerror}")


def _poll_upstreams() -> None:
    """Ask each usable server what it serves.

    `state` is online / offline / unknown -- never assumed online. A server that
    has never answered is `unknown`, which is a different thing from one that
    answered and then stopped.
    """
    for u in UPSTREAMS:
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
        if r.status != 200:
            raise RuntimeError(f"HTTP {r.status}")
    except Exception as exc:
        # Keep the last known model list: it describes what this node CAN serve,
        # which does not stop being true because the runtime is restarting.
        LOCAL_STATE.update(state="offline", error=str(exc)[:120])
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
                       last_seen=time.time())


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
    _auto_choice = None
    _route_why = None
    # The prefix of the body already read, to learn the model. Forwarded ahead
    # of the rest of the stream, so peeking costs a buffer and never a re-read.
    _peeked = b""
    # The path with any /u/<server> prefix removed. Set during resolution and
    # used for BOTH destinations: when auto picks this node the prefix still has
    # to go, or the local runtime is asked for a path it has never heard of.
    _route_path = None

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

    def _json(self, code: int, obj) -> None:
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        # If we are going to close, SAY so. Closing silently gives a client
        # reusing the connection a broken pipe instead of a clean retry.
        if self.close_connection:
            self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def _err(self, code: int, message: str, kind: str = "invalid_request") -> None:
        # OpenAI-compatible error shape, so a client library surfaces it.
        self._drain_body()
        self.close_connection = True
        self._json(code, {"error": {"message": message, "type": kind,
                                    "code": code}})

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
        path = self.path.split("?", 1)[0]
        if path.startswith("/u/"):
            return self._proxy(method, remote=True)
        if any(path == p.rstrip("/") or path.startswith(p) for p in OWNED):
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

    def _owned(self, method: str, path: str) -> None:
        if path == "/auth/login" and method == "GET":
            return self._login()
        if path == "/auth/callback" and method == "GET":
            return self._callback()
        if path == "/auth/logout":
            return self._logout()
        if path == "/api/me":
            return self._me()
        if path == "/api/gateway-health":
            return self._health()
        if path == "/api/stats":
            return self._stats()
        if path == "/api/servers":
            return self._servers()
        if path == "/api/usage":
            return self._usage_api()
        if path == "/api/keys":
            if method == "GET":
                return self._list_keys()
            if method == "POST":
                return self._mint()
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
        if not self._viewer():
            return
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
        self.send_response(r.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _servers(self) -> None:
        """Every registered server and what it serves.

        Session OR key, like /api/stats: this says which GPUs hold which models,
        which is what someone needs before choosing a base URL. It is never
        public -- it names internal hosts.
        """
        if not self._viewer():
            return
        # This node is described from its own PROBE, not asserted. The page used
        # to draw it as permanently online, which is the one server whose state
        # it could not actually have known.
        out = [{"id": _ups.LOCAL, "base_url": None, "enabled": True,
                "note": "this machine", "gpus": None, "problems": [],
                "needs_key": True, "state": LOCAL_STATE.get("state") or "unknown",
                "models": LOCAL_STATE.get("models") or [],
                "error": LOCAL_STATE.get("error"),
                "last_seen": LOCAL_STATE.get("last_seen"),
                "route": "/u/" + _ups.LOCAL + "/v1"}]
        for u in UPSTREAMS:
            st = UP_STATE.get(u.id) or {"state": "unknown", "models": [],
                                        "error": None, "last_seen": None}
            row = u.public()
            row.update(state=st.get("state", "unknown"), models=st.get("models") or [],
                       error=st.get("error"), last_seen=st.get("last_seen"),
                       # The path a client points at, so the page never has to
                       # construct it and get it subtly wrong.
                       route="/u/" + u.id + "/v1")
            out.append(row)
        self._json(200, {
            "servers": out, "poll_seconds": UPSTREAM_POLL_SECONDS,
            "registry_configured": bool(REGISTRY_PATH),
            # The plain path IS the virtual server, so the page can say so
            # rather than hard-coding a convention that might change.
            "default_route": "/v1",
            "auto_route": "/u/" + _ups.AUTO + "/v1",
            "balanced_paths": list(_ups.BALANCED_PATHS),
            "peek_bytes": _peek.PEEK_BYTES,
            "routing": dict(ROUTE_WHY)})

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
        who = self._viewer()
        if not who:
            return
        query = self.path.split("?", 1)[1] if "?" in self.path else ""
        mine = "mine=1" in query
        self._json(200, {
            "leaderboard": STORE.leaderboard(),
            "usage": STORE.usage(sub=who["sub"] if mine else None),
            "scope": "mine" if mine else "everyone",
            "you": who["sub"],
            "is_admin": who["is_admin"]})

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
                last = STORE.last_upstream(kid)
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
        if self._body_left <= 0:
            # No Content-Length either: a chunked body is left alone rather
            # than reframed, so nothing about the existing pass-through changes.
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

        resolved = _ups.route(raw, UPSTREAMS)
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
        st = UP_STATE.get(pinned.id) or {}
        if st.get("state") == "offline":
            seen = (f" (last seen {int(time.time() - st['last_seen'])}s ago)"
                    if st.get("last_seen") else " and has not been seen")
            self._err(503, f"{pinned.id} is not answering" + seen,
                      "upstream_offline")
            return ANSWERED
        self._decided(pinned.id, "pinned")
        return pinned

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
        chosen, why = _ups.pick_auto(
            UPSTREAMS, UP_STATE, self._last_server(), model=model,
            local_online=self._local_online(),
            local_models=LOCAL_STATE.get("models"))
        if why == "blind" and conclusive:
            # Told apart on purpose: `unnamed` means the request really has no
            # model field, `blind` means the peek ran out of buffer. One is a
            # client's choice, the other is a tuning problem here.
            why = "unnamed"

        if chosen is None:
            self._decided("none", "refused")
            if model is None:
                self._err(503, "cannot tell which model this request asks for, "
                               "and this node's runtime is not answering",
                          "no_upstream")
                return ANSWERED
            where = _ups.servers_for(UPSTREAMS, UP_STATE, model)
            if where:
                self._err(503, f"{model} is served by "
                               f"{', '.join(where)}, which is not answering",
                          "upstream_offline")
            else:
                # 404, not 503: nothing here serves it, so waiting will not help.
                self._err(404, f"no server on this node serves {model}",
                          "model_not_found")
            return ANSWERED

        self._decided(chosen, why)
        if chosen == _ups.LOCAL:
            return _ups.LOCAL
        for u in UPSTREAMS:
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
        self.wfile.write(body)

    # --- the proxy ---------------------------------------------------------
    def _proxy(self, method: str, remote: bool = False) -> None:
        row = self._authenticate_key(self._bearer())
        if row is None:
            return self._refuse_key()

        self._auth_key_id = row.key_id
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
            LAST_SERVER[row.key_id] = target.id if target else _ups.LOCAL

        started = time.time()
        acc = _usage.StreamAccountant()
        status = 0
        streamed = False
        try:
            status, streamed = self._relay(method, acc, target)
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
            u = acc.result()
            try:
                STORE.record_usage({
                    "ts": started, "key_id": row.key_id, "sub": row.sub,
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

    def _relay(self, method: str, acc, target=None) -> tuple[int, bool]:
        query = self.path.split("?", 1)
        suffix = ("?" + query[1]) if len(query) == 2 else ""
        if target is None:
            scheme, host, port = "http", CFG.upstream_host, CFG.upstream_port
            # _route_path is set whenever the request arrived under /u/, which
            # includes auto choosing THIS node.
            path = (self._route_path or query[0]) + suffix
            key = INTERNAL_KEY
        else:
            scheme, host, port, base = _ups.target(
                target, self._route_path or self._strip_prefix(query[0]))
            path = base + suffix
            key = UP_KEYS.get(target.id, "")
        cls = (http.client.HTTPSConnection if scheme == "https"
               else http.client.HTTPConnection)
        up = cls(host, port, timeout=900)
        try:
            self._send_upstream_request(up, method, path, host, port, key)
            r = up.getresponse()
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
