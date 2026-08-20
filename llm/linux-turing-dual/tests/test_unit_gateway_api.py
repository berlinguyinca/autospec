"""The key-management surface, exercised over real HTTP against a real handler.

What this covers: mint, list, revoke, usage, authorisation by group, and the
HTTPS requirement. What it does NOT cover is the OAuth round-trip itself -- that
needs a browser and a human at the identity provider, so the session is INJECTED
here. Everything downstream of "a verified token produced this session" is real:
a real socket, a real handler, a real store.

The tests assert the authorisation rules in both directions, because a
permission check that has only ever been tested from the allowed side is a
permission check nobody has tested.
"""
import http.client
import json
import threading
from http.server import ThreadingHTTPServer

import pytest

from conftest import load_script

gw = load_script("gateway")
keys = load_script("keys")


@pytest.fixture
def node(tmp_path):
    """A gateway serving on an ephemeral port, with a store and no upstream."""
    from keystore import KeyStore
    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    gw.STORE = store
    gw.CFG.user_group = "llm-users"
    gw.CFG.admin_group = "llm-admins"

    srv = ThreadingHTTPServer(("127.0.0.1", 0), gw.Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield srv, store
    srv.shutdown()


def _session(sub, groups):
    import secrets
    sid = secrets.token_urlsafe(8)
    gw._SESSIONS[sid] = {"sub": sub, "email": f"{sub}@example.org",
                         "name": sub, "groups": groups,
                         "created": __import__("time").time()}
    return sid


def call(srv, method, path, sid=None, body=None, https=True):
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    h = {"Content-Type": "application/json"}
    if https:
        h["X-Forwarded-Proto"] = "https"
    if sid:
        h["Cookie"] = f"{gw.SESSION_COOKIE}={sid}"
    c.request(method, path, json.dumps(body) if body is not None else None, h)
    r = c.getresponse()
    raw = r.read()
    c.close()
    try:
        return r.status, json.loads(raw or b"{}")
    except ValueError:
        return r.status, {"raw": raw[:200]}


# --- identity ---------------------------------------------------------------

def test_me_without_a_session_is_unauthenticated(node):
    srv, _ = node
    code, d = call(srv, "GET", "/api/me")
    assert code == 200 and d["authenticated"] is False


def test_a_member_of_no_group_is_authenticated_but_authorised_for_nothing(node):
    srv, _ = node
    sid = _session("sub-nobody", [])
    code, d = call(srv, "GET", "/api/me", sid)
    assert code == 200
    assert d["authenticated"] is True
    assert d["may_mint"] is False and d["is_admin"] is False
    # The page must be able to tell the user WHICH group they need.
    assert d["required_group"] == "llm-users"


def test_group_membership_in_a_HEADER_grants_nothing(node):
    """Groups come from the verified token only. This is the exact failure the
    operator's own services already have a test for."""
    srv, _ = node
    sid = _session("sub-nobody", [])
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/api/me", None, {
        "Cookie": f"{gw.SESSION_COOKIE}={sid}",
        "X-Forwarded-Proto": "https",
        # Attacker-supplied, and all legal header names -- a colon is not
        # allowed in one, so the literal claim name cannot even be sent.
        "X-Cognito-Groups": "llm-admins",
        "X-Groups": "llm-admins",
        "Groups": "llm-admins",
        "X-Amzn-Oidc-Groups": "llm-admins"})
    r = c.getresponse()
    d = json.loads(r.read())
    c.close()
    assert d["is_admin"] is False and d["may_mint"] is False


# --- minting ----------------------------------------------------------------

def test_minting_requires_a_session(node):
    srv, _ = node
    code, _ = call(srv, "POST", "/api/keys", body={"label": "x"})
    assert code == 401


def test_minting_requires_the_group(node):
    srv, _ = node
    sid = _session("sub-nobody", [])
    code, d = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    assert code == 403
    # The refusal names the group, so the user can act on it.
    assert "llm-users" in d["error"]["message"]


def test_minting_is_refused_over_plain_http(node):
    """A key minted over cleartext is a key already disclosed."""
    srv, _ = node
    sid = _session("sub-alice", ["llm-users"])
    code, d = call(srv, "POST", "/api/keys", sid, {"label": "x"}, https=False)
    assert code == 400 and d["error"]["type"] == "insecure_transport"


def test_a_member_mints_a_usable_key_shown_once(node):
    srv, store = node
    sid = _session("sub-alice", ["llm-users"])
    code, d = call(srv, "POST", "/api/keys", sid, {"label": "laptop"})
    assert code == 201
    assert d["shown_once"] is True
    assert keys.parse(d["key"]) is not None
    # It authenticates immediately, without waiting for any sync.
    import time
    assert store.authenticate(d["key"], time.time()) is not None
    # And listing it back never returns the secret again.
    code, listed = call(srv, "GET", "/api/keys", sid)
    assert code == 200
    assert all("key" not in k for k in listed["keys"])
    assert d["key"] not in json.dumps(listed)


def test_listing_shows_only_your_own_keys(node):
    srv, _ = node
    a = _session("sub-alice", ["llm-users"])
    b = _session("sub-bob", ["llm-users"])
    call(srv, "POST", "/api/keys", a, {"label": "a"})
    call(srv, "POST", "/api/keys", b, {"label": "b"})
    _, la = call(srv, "GET", "/api/keys", a)
    assert {k["sub"] for k in la["keys"]} == {"sub-alice"}
    # Asking for everyone's is ignored unless you are an admin.
    _, la_all = call(srv, "GET", "/api/keys?all=1", a)
    assert la_all["all_users"] is False
    assert {k["sub"] for k in la_all["keys"]} == {"sub-alice"}


def test_an_admin_can_see_every_key(node):
    srv, _ = node
    a = _session("sub-alice", ["llm-users"])
    adm = _session("sub-admin", ["llm-admins"])
    call(srv, "POST", "/api/keys", a, {"label": "a"})
    _, d = call(srv, "GET", "/api/keys?all=1", adm)
    assert d["all_users"] is True
    assert "sub-alice" in {k["sub"] for k in d["keys"]}


# --- revocation -------------------------------------------------------------

def test_revoking_your_own_key_takes_effect_immediately(node):
    srv, store = node
    import time
    sid = _session("sub-alice", ["llm-users"])
    _, minted = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    assert store.authenticate(minted["key"], time.time()) is not None
    code, d = call(srv, "DELETE", "/api/keys/" + minted["key_id"], sid)
    assert code == 200
    # The response promises immediacy; assert the promise is true.
    assert store.authenticate(minted["key"], time.time()) is None


def test_you_cannot_revoke_someone_elses_key(node):
    srv, store = node
    import time
    a = _session("sub-alice", ["llm-users"])
    b = _session("sub-bob", ["llm-users"])
    _, mine = call(srv, "POST", "/api/keys", b, {"label": "bobs"})
    code, _ = call(srv, "DELETE", "/api/keys/" + mine["key_id"], a)
    assert code == 404
    assert store.authenticate(mine["key"], time.time()) is not None


def test_an_admin_can_revoke_anyones_key(node):
    srv, store = node
    import time
    b = _session("sub-bob", ["llm-users"])
    adm = _session("sub-admin", ["llm-admins"])
    _, k = call(srv, "POST", "/api/keys", b, {"label": "bobs"})
    code, _ = call(srv, "DELETE", "/api/keys/" + k["key_id"], adm)
    assert code == 200
    assert store.authenticate(k["key"], time.time()) is None


def test_revocation_is_refused_over_plain_http(node):
    srv, _ = node
    sid = _session("sub-alice", ["llm-users"])
    _, k = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    code, d = call(srv, "DELETE", "/api/keys/" + k["key_id"], sid, https=False)
    assert code == 400 and d["error"]["type"] == "insecure_transport"


# --- usage ------------------------------------------------------------------

def test_usage_is_scoped_and_admins_see_everyone(node):
    srv, store = node
    a = _session("sub-alice", ["llm-users"])
    adm = _session("sub-admin", ["llm-admins"])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "a"})
    store.record_usage({"ts": 1_800_000_000.0, "key_id": ka["key_id"],
                        "sub": "sub-alice", "model": "m", "prompt_tokens": 10,
                        "completion_tokens": 5, "status_code": 200})
    _, mine = call(srv, "GET", "/api/usage", a)
    assert [u["sub"] for u in mine["usage"]] == ["sub-alice"]
    _, all_of_it = call(srv, "GET", "/api/usage?all=1", adm)
    assert all_of_it["all_users"] is True


def test_usage_requires_a_session(node):
    srv, _ = node
    code, _ = call(srv, "GET", "/api/usage")
    assert code == 401


# --- the login callback -----------------------------------------------------

def test_the_callback_refuses_an_unknown_state(node):
    """CSRF protection: a callback whose state this process never issued must be
    refused, whatever code it carries."""
    srv, _ = node
    code, d = call(srv, "GET", "/auth/callback?code=abc&state=never-issued")
    assert code == 400
    assert "state" in d["error"]["message"]


def test_the_callback_refuses_missing_parameters(node):
    srv, _ = node
    assert call(srv, "GET", "/auth/callback")[0] == 400
    assert call(srv, "GET", "/auth/callback?code=abc")[0] == 400


def test_an_unknown_endpoint_under_api_is_404_not_proxied(node):
    srv, _ = node
    code, _ = call(srv, "GET", "/api/keys/../secret")
    assert code in (400, 404)
