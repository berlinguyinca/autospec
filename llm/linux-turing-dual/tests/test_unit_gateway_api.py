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

from nodescripts import load_script

gw = load_script("gateway")
keys = load_script("keys")


@pytest.fixture
def node(tmp_path):
    """A gateway serving on an ephemeral port, with a store and no upstream."""
    from keystore import KeyStore
    store = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    store.migrate_local()
    gw.STORE = store
    gw.CFG.user_group = "*"          # the configured policy: the pool is the audience
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


def test_any_authenticated_pool_member_may_mint_when_open(node):
    """QT_COGNITO_USER_GROUP="*": the pool is the audience, so group membership
    is not consulted for minting."""
    srv, _ = node
    sid = _session("sub-nobody", [])
    code, d = call(srv, "GET", "/api/me", sid)
    assert code == 200
    assert d["authenticated"] is True
    assert d["may_mint"] is True
    assert d["is_admin"] is False        # admin is NOT opened by the same switch


def test_naming_a_group_narrows_minting_again(node):
    """The mechanism has to still work when it is used -- re-tightening should be
    a config change, not a code change."""
    srv, _ = node
    gw.CFG.user_group = "llm-users"
    try:
        outsider = _session("sub-outsider", [])
        member = _session("sub-member", ["llm-users"])
        assert call(srv, "GET", "/api/me", outsider)[1]["may_mint"] is False
        assert call(srv, "GET", "/api/me", member)[1]["may_mint"] is True
        # And the refusal names the group so the user can act on it.
        code, d = call(srv, "POST", "/api/keys", outsider, {"label": "x"})
        assert code == 403 and "llm-users" in d["error"]["message"]
    finally:
        gw.CFG.user_group = "*"


def test_an_admin_group_member_is_still_distinguished(node):
    srv, _ = node
    adm = _session("sub-admin", ["llm-admins"])
    d = call(srv, "GET", "/api/me", adm)[1]
    assert d["is_admin"] is True and d["may_mint"] is True


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
    # ADMIN is the group-gated privilege, so it is the one a forged header would
    # be trying to obtain. may_mint is true here by POLICY ("*"), not by header,
    # which is why this asserts on is_admin.
    assert d["is_admin"] is False
    assert d["groups"] == []


# --- minting ----------------------------------------------------------------

def test_minting_requires_a_session(node):
    srv, _ = node
    code, _ = call(srv, "POST", "/api/keys", body={"label": "x"})
    assert code == 401


def test_minting_is_open_to_the_pool_as_configured(node):
    srv, _ = node
    sid = _session("sub-nobody", [])
    code, d = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    assert code == 201


def test_minting_is_refused_over_plain_http(node):
    """A key minted over cleartext is a key already disclosed."""
    srv, _ = node
    sid = _session("sub-alice", [])
    code, d = call(srv, "POST", "/api/keys", sid, {"label": "x"}, https=False)
    assert code == 400 and d["error"]["type"] == "insecure_transport"


def test_a_member_mints_a_usable_key_shown_once(node):
    srv, store = node
    sid = _session("sub-alice", [])
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
    a = _session("sub-alice", [])
    b = _session("sub-bob", [])
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
    a = _session("sub-alice", [])
    adm = _session("sub-admin", ["llm-admins"])
    call(srv, "POST", "/api/keys", a, {"label": "a"})
    _, d = call(srv, "GET", "/api/keys?all=1", adm)
    assert d["all_users"] is True
    assert "sub-alice" in {k["sub"] for k in d["keys"]}


# --- revocation -------------------------------------------------------------

def test_revoking_your_own_key_takes_effect_immediately(node):
    srv, store = node
    import time
    sid = _session("sub-alice", [])
    _, minted = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    assert store.authenticate(minted["key"], time.time()) is not None
    code, d = call(srv, "DELETE", "/api/keys/" + minted["key_id"], sid)
    assert code == 200
    # The response promises immediacy; assert the promise is true.
    assert store.authenticate(minted["key"], time.time()) is None


def test_you_cannot_revoke_someone_elses_key(node):
    srv, store = node
    import time
    a = _session("sub-alice", [])
    b = _session("sub-bob", [])
    _, mine = call(srv, "POST", "/api/keys", b, {"label": "bobs"})
    code, _ = call(srv, "DELETE", "/api/keys/" + mine["key_id"], a)
    assert code == 404
    assert store.authenticate(mine["key"], time.time()) is not None


def test_an_admin_can_revoke_anyones_key(node):
    srv, store = node
    import time
    b = _session("sub-bob", [])
    adm = _session("sub-admin", ["llm-admins"])
    _, k = call(srv, "POST", "/api/keys", b, {"label": "bobs"})
    code, _ = call(srv, "DELETE", "/api/keys/" + k["key_id"], adm)
    assert code == 200
    assert store.authenticate(k["key"], time.time()) is None


def test_revocation_is_refused_over_plain_http(node):
    srv, _ = node
    sid = _session("sub-alice", [])
    _, k = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    code, d = call(srv, "DELETE", "/api/keys/" + k["key_id"], sid, https=False)
    assert code == 400 and d["error"]["type"] == "insecure_transport"


# --- usage ------------------------------------------------------------------

def _spend(store, key_id, sub, prompt, completion, ts=1_800_000_000.0):
    store.record_usage({"ts": ts, "key_id": key_id, "sub": sub, "model": "m",
                        "prompt_tokens": prompt, "completion_tokens": completion,
                        "status_code": 200})


def test_the_scoreboard_ranks_users_by_tokens(node):
    """A leaderboard that showed only your own row would not be one."""
    srv, store = node
    a = _session("sub-alice", [])
    b = _session("sub-bob", [])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "a"})
    _, kb = call(srv, "POST", "/api/keys", b, {"label": "b"})
    _spend(store, ka["key_id"], "sub-alice", 10, 5)      # 15 total
    _spend(store, kb["key_id"], "sub-bob", 100, 50)      # 150 total
    # Asked by ALICE, and Bob must still be on the board -- above her.
    d = call(srv, "GET", "/api/usage", a)[1]
    board = d["leaderboard"]
    assert [r["sub"] for r in board] == ["sub-bob", "sub-alice"]
    assert board[0]["total_tokens"] == 150 and board[1]["total_tokens"] == 15
    assert d["you"] == "sub-alice"           # so the page can highlight your row


def test_the_scoreboard_names_the_person_not_just_the_subject(node):
    srv, store = node
    store.upsert_user("sub-alice", "alice@example.org", "Alice Example")
    a = _session("sub-alice", [])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "a"})
    _spend(store, ka["key_id"], "sub-alice", 7, 3)
    row = call(srv, "GET", "/api/usage", a)[1]["leaderboard"][0]
    assert row["display_name"] == "Alice Example"
    assert row["email"] == "alice@example.org"


def test_a_key_minted_before_its_owner_signed_in_still_ranks(node):
    """The break-glass mints without a user row; the board must not drop it."""
    srv, store = node
    a = _session("sub-alice", [])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "a"})
    _spend(store, ka["key_id"], "sub-unseen", 5, 5)
    subs = [r["sub"] for r in call(srv, "GET", "/api/usage", a)[1]["leaderboard"]]
    assert "sub-unseen" in subs


def test_the_key_detail_table_names_the_owner_and_ranks_by_tokens(node):
    srv, store = node
    store.upsert_user("sub-bob", "bob@example.org", "Bob")
    a = _session("sub-alice", [])
    b = _session("sub-bob", [])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "small"})
    _, kb = call(srv, "POST", "/api/keys", b, {"label": "big"})
    _spend(store, ka["key_id"], "sub-alice", 1, 1)
    _spend(store, kb["key_id"], "sub-bob", 90, 10)
    rows = call(srv, "GET", "/api/usage", a)[1]["usage"]
    assert rows[0]["key_id"] == kb["key_id"]          # ranked by tokens
    assert rows[0]["display_name"] == "Bob"           # named, not an opaque id
    assert rows[0]["label"] == "big"


def test_mine_narrows_the_detail_table_to_the_caller(node):
    srv, store = node
    a = _session("sub-alice", [])
    b = _session("sub-bob", [])
    _, ka = call(srv, "POST", "/api/keys", a, {"label": "a"})
    _, kb = call(srv, "POST", "/api/keys", b, {"label": "b"})
    _spend(store, ka["key_id"], "sub-alice", 5, 5)
    _spend(store, kb["key_id"], "sub-bob", 5, 5)
    d = call(srv, "GET", "/api/usage?mine=1", a)[1]
    assert {u["sub"] for u in d["usage"]} == {"sub-alice"}
    assert d["scope"] == "mine"
    # The board is still everyone: narrowing the detail must not narrow the board.
    assert {r["sub"] for r in d["leaderboard"]} == {"sub-alice", "sub-bob"}


def test_the_leaderboard_is_public_but_the_key_table_is_not(node):
    """Anyone may read the scoreboard; only a signed-in caller gets the per-key
    detail, which names each person's keys rather than describing the node."""
    srv, store = node
    key_id = store.mint("sub-alice", "k")[1].key_id
    _spend(store, key_id, "sub-alice", 100, 200)

    code, anon = call(srv, "GET", "/api/usage")
    assert code == 200
    assert anon["leaderboard"], "the public scoreboard must not be empty"
    assert anon["usage"] == []
    assert anon["scope"] == "public"

    code, mine = call(srv, "GET", "/api/usage", _session("sub-alice", []))
    assert code == 200
    assert mine["usage"], "a signed-in caller still gets the key table"


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


def test_a_key_that_can_infer_can_also_read_stats(node, monkeypatch):
    """The proxy and the stats endpoint must answer the SAME question about a
    credential. They diverged once, when a migration hatch was added to the proxy
    and not to stats, so one credential could run inference but not the page."""
    srv, store = node
    import time
    sid = _session("sub-alice", [])
    _, minted = call(srv, "POST", "/api/keys", sid, {"label": "x"})
    key = minted["key"]
    # The proxy's authenticator accepts it...
    assert store.authenticate(key, time.time()) is not None
    # ...and so does the stats endpoint, reached with no session at all.
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/api/stats", None,
              {"Authorization": "Bearer " + key, "X-Forwarded-Proto": "https"})
    r = c.getresponse(); r.read(); c.close()
    # 502 is fine here -- there is no dashboard behind this test gateway. What
    # matters is that it got PAST authentication rather than answering 401.
    assert r.status != 401


def test_an_api_key_can_read_the_scoreboard_as_its_owner(node):
    """One notion of an authenticated reader: /api/stats took a key while
    /api/usage demanded a session, which drew an arbitrary line through the read
    endpoints."""
    srv, store = node
    sid = _session("sub-alice", [])
    _, minted = call(srv, "POST", "/api/keys", sid, {"label": "script"})
    _spend(store, minted["key_id"], "sub-alice", 20, 10)
    c = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=10)
    c.request("GET", "/api/usage", None,
              {"Authorization": "Bearer " + minted["key"],
               "X-Forwarded-Proto": "https"})
    r = c.getresponse(); d = json.loads(r.read()); c.close()
    assert r.status == 200
    assert d["you"] == "sub-alice"        # reads as its owner
    assert d["is_admin"] is False         # a key never carries admin
    assert d["leaderboard"][0]["total_tokens"] == 30


def test_the_fleet_is_public_without_saying_where_anything_lives(node):
    """Capability and load are public; addresses and owners are not.

    Asserted as ABSENCE on the payload a stranger receives, because that is the
    only form the guarantee has: publicview.py projects by allow-list, so a field
    added to the private payload later is caught here rather than published.
    """
    srv, _ = node
    code, anon = call(srv, "GET", "/api/servers")
    assert code == 200
    assert anon["public"] is True
    assert anon["servers"], "the public fleet view must still name the servers"
    for row in anon["servers"]:
        assert "id" in row and "state" in row and "models" in row
        for forbidden in ("base_url", "owner", "owner_name", "error",
                          "problems", "needs_key"):
            assert forbidden not in row, f"{forbidden} leaked to a stranger"
    assert "you" not in anon

    code, full = call(srv, "GET", "/api/servers", _session("sub-alice", []))
    assert code == 200
    assert "base_url" in full["servers"][0], "the private view keeps addresses"
    assert full["you"]["sub"] == "sub-alice"
