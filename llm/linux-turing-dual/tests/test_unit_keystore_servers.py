"""Server identities, enrolment, and the measurements the scheduler ranks on.

The test that matters most here is the one asserting a server credential is not
a user key and a user key is not a server credential -- in every direction. One
`authenticate()` for everything is exactly how a credential that may only offer
capacity silently becomes one that may spend it.
"""
import time

import pytest

from conftest import load_script

keys = load_script("keys")
ks = load_script("keystore")
up = load_script("upstreams")


@pytest.fixture
def store(tmp_path):
    s = ks.KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    s.migrate_local()
    s.upsert_user("sub-a", email="a@example.org", name="A")
    s.upsert_user("sub-b", email="b@example.org", name="B")
    return s


def attach(store, server_id="box", sub="sub-a", **kw):
    tok = store.enrol_token(sub, server_id, **kw)
    return store.redeem_token(tok)


# --- enrolment --------------------------------------------------------------

def test_an_enrolment_token_works_exactly_once(store):
    tok = store.enrol_token("sub-a", "box")
    assert tok.startswith("qte_")
    first = store.redeem_token(tok)
    assert first is not None and first[0] == "box"
    assert store.redeem_token(tok) is None


def test_an_expired_token_is_refused(store):
    now = time.time()
    tok = store.enrol_token("sub-a", "box", now=now)
    assert store.redeem_token(tok, now=now + ks.ENROL_TTL_SECONDS + 1) is None
    # And is still good just inside the window, or the test would pass against a
    # token that never works at all.
    tok2 = store.enrol_token("sub-a", "box2", now=now)
    assert store.redeem_token(tok2, now=now + 10) is not None


def test_a_forged_token_with_a_real_id_is_refused(store):
    real = store.enrol_token("sub-a", "box")
    token_id = keys.public_id(real, keys.PREFIX_ENROL)
    forged = f"qte_{token_id}_" + "a" * 32
    assert store.redeem_token(forged) is None
    assert store.redeem_token(real) is not None      # the real one still works


def test_the_credential_is_stored_only_as_a_hash(store):
    _, cred = attach(store)
    secret = cred.split("_")[-1]
    with store._conn() as c:
        dumped = str([tuple(r) for r in c.execute("SELECT * FROM servers")])
    assert secret not in dumped


# --- the namespaces, in every direction -------------------------------------

def test_a_server_credential_is_not_a_user_key_and_a_user_key_is_not_a_server(store):
    _, cred = attach(store)
    user, _ = store.mint("sub-a", label="k")
    assert store.authenticate(cred, time.time()) is None
    assert store.authenticate_server(user) is None
    # Each still works as itself, or this proves only that both are broken.
    assert store.authenticate(user, time.time()) is not None
    assert store.authenticate_server(cred) is not None


def test_an_enrolment_token_is_not_a_server_credential(store):
    tok = store.enrol_token("sub-a", "box")
    assert store.authenticate_server(tok) is None
    assert store.authenticate(tok, time.time()) is None


def test_a_revoked_credential_stops_authenticating(store):
    _, cred = attach(store)
    assert store.authenticate_server(cred) is not None
    assert store.revoke_server("box", sub="sub-a") is True
    assert store.authenticate_server(cred) is None


# --- ids --------------------------------------------------------------------

@pytest.mark.parametrize("bad", ["local", "auto"])
def test_reserved_ids_are_refused(store, bad):
    with pytest.raises(ValueError) as exc:
        store.enrol_token("sub-a", bad)
    assert "reserved" in str(exc.value)
    # Nothing was created by the attempt, and a name that merely CONTAINS the
    # reserved word is still fine.
    assert store.servers() == []
    assert store.enrol_token("sub-a", bad + "-2").startswith("qte_")


def test_the_reserved_list_matches_the_router_s(store):
    """Two copies of one fact, so they are asserted equal rather than trusted."""
    assert tuple(ks.RESERVED_SERVER_IDS) == tuple(up.RESERVED)


@pytest.mark.parametrize("bad", ["Box", "b ox", "-box", "x" * 32, "", "b/x"])
def test_an_id_that_is_not_a_path_segment_is_refused(store, bad):
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", bad)
    assert store.servers() == []
    # The boundary is a real one: 31 characters is the longest legal id.
    assert store.enrol_token("sub-a", "x" * 31).startswith("qte_")


def test_a_duplicate_id_is_refused_including_one_still_pending(store):
    first = store.enrol_token("sub-a", "box")
    with pytest.raises(ValueError):
        store.enrol_token("sub-b", "box")          # pending, not yet redeemed
    # The refusal did not consume the first token.
    assert store.redeem_token(first) is not None
    store.redeem_token(store.enrol_token("sub-a", "box2"))
    with pytest.raises(ValueError):
        store.enrol_token("sub-b", "box2")         # live
    assert {s["server_id"] for s in store.servers()} == {"box", "box2"}


def test_a_revoked_id_may_be_reused(store):
    """A box that was detached and is being re-attached is the common case;
    refusing its old name would force people to invent box2."""
    attach(store, "box")
    store.revoke_server("box", sub="sub-a")
    assert attach(store, "box") is not None


# --- static and tunnelled ---------------------------------------------------

def test_a_static_server_needs_an_address_and_a_tunnelled_one_must_not_have_one(store):
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", "s1", kind="static")
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", "s2", kind="static", base_url="box:8000/v1")
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", "t1", base_url="http://box:8000/v1")
    ok = store.enrol_token("sub-a", "s3", kind="static",
                           base_url="http://box.invalid:8000/v1")
    assert store.redeem_token(ok) is not None


def test_a_tunnelled_server_reports_no_address_even_if_one_were_stored(store):
    """There is no address, and a field for one would invite someone to fill it
    in -- at which point the node would dial a box that expects to dial out."""
    attach(store, "box")
    row = store.server("box")
    assert row.kind == "tunnel" and row.as_public()["base_url"] is None


def test_an_unknown_kind_is_refused(store):
    with pytest.raises(ValueError) as exc:
        store.enrol_token("sub-a", "box", kind="carrier-pigeon")
    assert "tunnel" in str(exc.value) and "static" in str(exc.value)
    assert store.servers() == []
    # 'file' is a real kind, but not one anybody may ATTACH -- it means "came
    # from the registry file", which this path is not.
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", "box", kind="file")


def test_a_dialled_server_may_carry_the_key_it_demands(store):
    """Otherwise the dashboard path is strictly weaker than the registry file for
    the one kind of server that needs a credential -- and the symptom is a server
    that looks online, reports its models, and then 401s on the first request."""
    tok = store.enrol_token("sub-a", "keyed", kind="static",
                            base_url="http://box.invalid:8000/v1", api_key="sk-abc")
    store.redeem_token(tok)
    assert store.upstream_keys() == {"keyed": "sk-abc"}
    row = [r for r in store.servers() if r["server_id"] == "keyed"][0]
    # Whether a key is held, never the key.
    assert row["has_key"] is True
    assert "sk-abc" not in str(store.servers())


def test_a_tunnelled_server_may_not_carry_a_key(store):
    """Not an oversight to be worked around: the pipe is an opaque byte stream, so
    the agent would have to parse and rewrite the request head to add a header.
    Binding the target to loopback is the answer, and it is stronger."""
    with pytest.raises(ValueError) as exc:
        store.enrol_token("sub-a", "box", api_key="sk-abc")
    assert "loopback" in str(exc.value)
    assert store.enrol_token("sub-a", "box").startswith("qte_")


def test_revoking_a_server_withdraws_its_upstream_key(store):
    tok = store.enrol_token("sub-a", "keyed", kind="static",
                            base_url="http://box.invalid:8000/v1", api_key="sk-abc")
    store.redeem_token(tok)
    store.revoke_server("keyed", sub="sub-a")
    assert store.upstream_keys() == {}


def test_the_upstream_key_column_is_added_to_an_older_database(tmp_path):
    """A node upgraded in place has the table already, and CREATE TABLE IF NOT
    EXISTS does not add a column to it."""
    import sqlite3
    path = str(tmp_path / "old.sqlite3")
    con = sqlite3.connect(path)
    con.executescript(
        "CREATE TABLE servers (server_id TEXT PRIMARY KEY, sub TEXT, kind TEXT "
        "NOT NULL, base_url TEXT, note TEXT, gpus TEXT, priority INTEGER NOT NULL "
        "DEFAULT 0, pool_member INTEGER NOT NULL DEFAULT 0, secret_hash TEXT, "
        "created_at TEXT NOT NULL, revoked_at TEXT, last_seen TEXT);")
    con.commit()
    con.close()
    older = ks.KeyStore(path, dsn=None)
    older.migrate_local()
    with older._conn() as c:
        cols = {r["name"] for r in c.execute("PRAGMA table_info(servers)")}
    assert "upstream_key" in cols
    older.upsert_user("sub-a", email="a@example.org", name="A")
    tok = older.enrol_token("sub-a", "keyed", kind="static",
                            base_url="http://b.invalid/v1", api_key="sk-x")
    assert older.redeem_token(tok) is not None
    assert older.upstream_keys() == {"keyed": "sk-x"}


# --- the pool and the tier --------------------------------------------------

def test_a_new_server_is_not_in_the_default_pool(store):
    _, cred = attach(store)
    assert store.authenticate_server(cred).pool_member is False


def test_promotion_is_what_puts_it_in_the_pool(store):
    _, cred = attach(store)
    assert store.set_pool_member("box", True) is True
    assert store.authenticate_server(cred).pool_member is True
    assert store.set_pool_member("box", False) is True
    assert store.authenticate_server(cred).pool_member is False


def test_priority_is_settable_and_bounded(store):
    attach(store)
    assert store.set_priority("box", 3) is True
    assert store.server("box").priority == 3
    for bad in (11, -11, 999):
        with pytest.raises(ValueError):
            store.set_priority("box", bad)
    assert store.server("box").priority == 3       # unchanged by the refusals


def test_updating_an_unknown_or_revoked_server_reports_failure(store):
    assert store.set_pool_member("nope", True) is False
    attach(store, "box")
    store.revoke_server("box", sub="sub-a")
    assert store.set_priority("box", 1) is False


# --- ownership --------------------------------------------------------------

def test_the_owner_may_revoke_and_a_stranger_may_not(store):
    attach(store)
    assert store.revoke_server("box", sub="sub-b") is False
    assert store.revoke_server("box", sub="sub-a") is True


def test_an_admin_may_revoke_anyone_s_server(store):
    attach(store)
    assert store.revoke_server("box", sub="sub-b", is_admin=True) is True


def test_a_file_configured_server_cannot_be_revoked_from_the_panel(store):
    """It is configuration. Revoking it would be undone by the next reload, and
    a button that silently does nothing is worse than no button."""
    with store._conn() as c:
        c.execute("INSERT INTO servers (server_id, sub, kind, base_url, "
                  "created_at) VALUES ('cfg', NULL, 'file', "
                  "'http://x.invalid/v1', '2026-01-01T00:00:00+00:00')")
    assert store.revoke_server("cfg", is_admin=True) is False


def test_listing_scopes_to_a_user_or_shows_everyone(store):
    attach(store, "boxa", "sub-a")
    attach(store, "boxb", "sub-b")
    assert {s["server_id"] for s in store.servers(sub="sub-a")} == {"boxa"}
    assert {s["server_id"] for s in store.servers()} == {"boxa", "boxb"}


def test_last_seen_is_recorded_for_the_panel(store):
    attach(store)
    assert store.server("box").last_seen is None
    store.touch_server("box", now=1_700_000_000.0)
    assert store.server("box").last_seen is not None


# --- measured throughput ----------------------------------------------------

def _usage(store, upstream, model, ptok, pms, ctok, cms, ts=None, truncated=False):
    store.record_usage({
        "ts": ts if ts is not None else time.time(), "key_id": "k", "sub": "sub-a",
        "model": model, "upstream": upstream, "endpoint": "/v1/chat/completions",
        "prompt_tokens": ptok, "completion_tokens": ctok, "cached_tokens": 0,
        "prompt_ms": pms, "predicted_ms": cms, "status_code": 200,
        "streamed": True, "truncated": truncated})


def test_throughput_is_measured_per_server_and_model(store):
    _usage(store, "boxa", "m1", 1000, 1000.0, 100, 1000.0)
    _usage(store, "boxb", "m1", 1000, 4000.0, 100, 4000.0)
    t = store.throughput()
    assert round(t[("boxa", "m1")]["prefill_rate"]) == 1000
    assert round(t[("boxb", "m1")]["prefill_rate"]) == 250
    # Faster box, higher rate: this is the ordering the scheduler needs.
    assert t[("boxa", "m1")]["prefill_rate"] > t[("boxb", "m1")]["prefill_rate"]


def test_a_model_never_served_there_falls_back_to_the_server_aggregate(store):
    _usage(store, "boxa", "m1", 1000, 1000.0, 100, 1000.0)
    t = store.throughput()
    assert ("boxa", "m2") not in t
    assert t[("boxa", None)]["samples"] == 1


def test_rates_differ_enough_by_model_to_justify_the_key(store):
    """A 9B and a 27B on one card differ by an order of magnitude, so a single
    per-server rate would misestimate every request."""
    _usage(store, "boxa", "small", 1000, 500.0, 10, 100.0)
    _usage(store, "boxa", "large", 1000, 5000.0, 10, 1000.0)
    t = store.throughput()
    assert t[("boxa", "small")]["prefill_rate"] > 5 * t[("boxa", "large")]["prefill_rate"]


def test_a_truncated_row_is_excluded(store):
    """Its counts were lost, so including it would look like a free request."""
    _usage(store, "boxa", "m1", 1000, 1000.0, 100, 1000.0)
    _usage(store, "boxa", "m1", 0, 1.0, 0, 0.0, truncated=True)
    assert store.throughput()[("boxa", "m1")]["samples"] == 1


def test_a_row_outside_the_window_is_excluded(store):
    now = time.time()
    _usage(store, "boxa", "m1", 1000, 1000.0, 100, 1000.0, ts=now - 40 * 86400)
    assert store.throughput(now=now) == {}
    assert store.throughput(now=now, window_seconds=60 * 86400) != {}


def test_an_unmeasured_server_is_simply_absent(store):
    """Absent means "try it and find out". Substituting a zero would rank it
    first forever; substituting infinity would mean it never runs again."""
    assert store.throughput() == {}
