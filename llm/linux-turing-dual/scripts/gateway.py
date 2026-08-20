#!/usr/bin/env python3
"""The authenticating gateway: per-user keys in, exact usage out.

WHERE THIS SITS

    client -> nginx -> THIS (127.0.0.1) -> llama.cpp (127.0.0.1)

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
 4. Token counts come from the response, so the request body is never parsed.
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
import oidc as _oidc            # noqa: E402
import usage as _usage          # noqa: E402
from keystore import KeyRow, KeyStore   # noqa: E402

CHUNK = 65536
SESSION_COOKIE = "qt_session"
SESSION_TTL = 12 * 3600
STATE_TTL = 600
JWKS_MIN_INTERVAL = 60          # never refetch more often than this
REFRESH_SECONDS = 30            # registry -> mirror; the revocation staleness bound
FLUSH_SECONDS = 30

# Paths this process owns. Everything else is proxied to the runtime.
OWNED = ("/auth/", "/api/keys", "/api/me", "/api/usage", "/api/gateway-health",
         "/api/stats")

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

# MIGRATION HATCH, meant to be removed.
#
# The node ran on one shared key before this gateway existed. That key is not in
# qtk_ format, so it cannot be stored as an ordinary key -- and clients
# configured against it must not break on the day the gateway lands. So it is
# accepted here, attributed to a single reserved key id, and its traffic is
# recorded like everyone else's.
#
# That accounting is the point: the usage panel shows exactly how much still
# arrives on the shared key, which is the signal for when retiring it is safe.
# Delete this, the two flags, and LEGACY_KEY_ID once that number is zero.
LEGACY_KEY = ""
LEGACY_SUB = ""
LEGACY_KEY_ID = "legacy-shared"


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
        """(may_mint, is_admin), from the VERIFIED token's groups only."""
        g = s.get("groups") or []
        is_admin = bool(CFG.admin_group) and CFG.admin_group in g
        may_mint = is_admin or (bool(CFG.user_group) and CFG.user_group in g)
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
        path = self.path.split("?", 1)[0]
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

        ONE place, used by both the proxy and the stats endpoint. They had
        diverged: the proxy honoured the legacy shared key and the stats endpoint
        did not, so the same credential could run inference but not read the
        dashboard. Two copies of an auth check is two answers to one question.
        """
        row = STORE.authenticate(presented, time.time()) if STORE else None
        if row is None and LEGACY_KEY and presented:
            import hmac
            if hmac.compare_digest(presented, LEGACY_KEY):
                row = KeyRow(key_id=LEGACY_KEY_ID, sub=LEGACY_SUB or "legacy")
        return row

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
        s = self._session()
        if s is None:
            if not self._authenticate_key(self._bearer()):
                self._drain_body()
                self.close_connection = True
                return self._json(401, {"error": {
                    "message": "sign in, or present an API key",
                    "type": "not_authenticated", "code": 401},
                    "login_configured": CFG.login_configured()})
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

    # --- keys --------------------------------------------------------------
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
        s = self._require_session()
        if not s:
            return
        _, is_admin = self._may(s)
        want_all = "all=1" in (self.path.split("?", 1)[1] if "?" in self.path else "")
        rows = STORE.usage(sub=None if (want_all and is_admin) else s["sub"])
        self._json(200, {"usage": rows, "all_users": bool(want_all and is_admin)})

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
    def _proxy(self, method: str) -> None:
        row = self._authenticate_key(self._bearer())
        if row is None:
            return self._refuse_key()

        started = time.time()
        acc = _usage.StreamAccountant()
        status = 0
        streamed = False
        try:
            status, streamed = self._relay(method, acc)
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
                    "model": u.model, "upstream": "local",
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

    def _relay(self, method: str, acc) -> tuple[int, bool]:
        up = http.client.HTTPConnection(CFG.upstream_host, CFG.upstream_port,
                                        timeout=900)
        try:
            self._send_upstream_request(up, method)
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

    def _send_upstream_request(self, up, method: str) -> None:
        up.putrequest(method, self.path, skip_host=True, skip_accept_encoding=True)
        for k, v in self.headers.items():
            if k.lower() in ("connection", "keep-alive", "transfer-encoding",
                             "authorization", "cookie", "host"):
                continue
            up.putheader(k, v)
        up.putheader("Host", f"{CFG.upstream_host}:{CFG.upstream_port}")
        # The client's credential is NEVER forwarded; the runtime has its own.
        if INTERNAL_KEY:
            up.putheader("Authorization", f"Bearer {INTERNAL_KEY}")
        up.endheaders()

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
    global INTERNAL_KEY, DASHBOARD_KEY, LEGACY_KEY, LEGACY_SUB
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

    if args.legacy_key_file:
        try:
            LEGACY_KEY = (open(args.legacy_key_file).readline() or "").strip()
        except OSError as exc:
            return f"cannot read the legacy key: {exc}"
        LEGACY_SUB = os.environ.get("QT_LEGACY_SUB", "legacy")
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
    ap.add_argument("--legacy-key-file",
                    help="the pre-gateway shared key; accepted and ACCOUNTED so "
                         "its remaining traffic is visible. Temporary.")
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

    STORE = KeyStore(args.mirror, dsn=dsn)
    STORE.migrate_local()
    if dsn:
        STORE.refresh()

    threading.Thread(target=_housekeeper, daemon=True).start()

    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"gateway on http://{args.host}:{args.port} -> "
          f"{CFG.upstream_host}:{CFG.upstream_port} "
          f"(registry {'configured' if dsn else 'absent'}"
          f"{', legacy key accepted' if LEGACY_KEY else ''}"
          f"{', stats fronted' if DASHBOARD_KEY else ''})", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
