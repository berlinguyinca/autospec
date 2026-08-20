"""The store. SQLite only here (dsn=None): the local mirror is the ENFORCEMENT
point, so it is the part that must be right even when the registry is gone.

No test touches PostgreSQL. A test that needs a live database is a test that
gets skipped in CI and then rots.
"""
import importlib.util
import pathlib
import sys
import time

import pytest

from conftest import load_script

SCRIPTS = pathlib.Path(__file__).resolve().parent.parent / "scripts"




keys = load_script("keys")
ks = load_script("keystore")


@pytest.fixture
def store(tmp_path):
    s = ks.KeyStore(str(tmp_path / "mirror.sqlite3"), dsn=None)
    s.migrate_local()
    s.upsert_user("sub-alice", "alice@example.org", "Alice")
    s.upsert_user("sub-bob", "bob@example.org", "Bob")
    return s


NOW = 1_800_000_000.0


def test_a_minted_key_authenticates(store):
    full, row = store.mint("sub-alice", label="laptop")
    assert row.sub == "sub-alice" and row.label == "laptop"
    got = store.authenticate(full, NOW)
    assert got is not None and got.key_id == row.key_id and got.sub == "sub-alice"


def test_the_secret_is_not_recoverable_from_the_store(store):
    full, row = store.mint("sub-alice")
    _, secret = keys.parse(full)
    listed = store.list_keys("sub-alice")
    assert all(secret not in (r.secret_hash or "") for r in listed)
    # And listing never hands back anything usable as a credential.
    assert all(keys.parse(r.key_id) is None for r in listed)


def test_a_revoked_key_fails_IMMEDIATELY_after_revoke_returns(store):
    """The failure that actually matters: a revoked key that still works because
    a cache has not refreshed."""
    full, row = store.mint("sub-alice")
    assert store.authenticate(full, NOW) is not None
    assert store.revoke(row.key_id, sub="sub-alice", is_admin=False) is True
    assert store.authenticate(full, NOW) is None


def test_revoking_one_key_leaves_the_others_working(store):
    a_full, a = store.mint("sub-alice", label="a")
    b_full, b = store.mint("sub-alice", label="b")
    store.revoke(a.key_id, sub="sub-alice", is_admin=False)
    assert store.authenticate(a_full, NOW) is None
    assert store.authenticate(b_full, NOW) is not None


def test_an_expired_key_fails(store):
    # Minted ON the injected clock: with the wall clock, a 1-day TTL would
    # already be in the past relative to NOW and the test would pass for the
    # wrong reason.
    full, row = store.mint("sub-alice", ttl_days=1, now=NOW)
    assert store.authenticate(full, NOW) is not None
    assert store.authenticate(full, NOW + 2 * 86400) is None


def test_a_user_cannot_revoke_someone_elses_key(store):
    _, bob_key = store.mint("sub-bob")
    assert store.revoke(bob_key.key_id, sub="sub-alice", is_admin=False) is False
    # ...and it still works, i.e. the refusal did not half-apply.
    assert store.list_keys("sub-bob")[0].revoked_at is None


def test_an_admin_can_revoke_anyones_key(store):
    _, bob_key = store.mint("sub-bob")
    assert store.revoke(bob_key.key_id, sub="sub-alice", is_admin=True) is True
    assert store.list_keys("sub-bob")[0].revoked_at is not None


def test_listing_is_scoped_to_one_subject_unless_admin(store):
    store.mint("sub-alice"); store.mint("sub-bob")
    assert {r.sub for r in store.list_keys("sub-alice")} == {"sub-alice"}
    assert {r.sub for r in store.list_keys("sub-alice", all_users=True)} == \
        {"sub-alice", "sub-bob"}


def test_unknown_and_malformed_keys_are_refused(store):
    assert store.authenticate("qtk_aaaaaaaaaaaa_" + "b" * 32, NOW) is None
    assert store.authenticate("garbage", NOW) is None
    assert store.authenticate("", NOW) is None


def test_a_right_key_id_with_a_wrong_secret_is_refused(store):
    full, row = store.mint("sub-alice")
    forged = f"{keys.PREFIX}_{row.key_id}_" + "z" * keys.SECRET_LEN
    assert store.authenticate(forged, NOW) is None


def test_authenticate_records_last_used(store):
    full, row = store.mint("sub-alice")
    assert store.list_keys("sub-alice")[0].last_used_at is None
    store.authenticate(full, NOW)
    assert store.list_keys("sub-alice")[0].last_used_at is not None


# --- usage ------------------------------------------------------------------

def _rec(store, key_id, sub, **over):
    r = {"event_id": None, "ts": NOW, "key_id": key_id, "sub": sub,
         "model": "qwen3.5-9b", "upstream": "local", "endpoint": "/v1/chat/completions",
         "prompt_tokens": 13, "completion_tokens": 12, "cached_tokens": 0,
         "prompt_ms": 60.0, "predicted_ms": 100.0, "status_code": 200,
         "streamed": True, "truncated": False}
    r.update(over)
    return r


def test_usage_is_recorded_and_summarised_per_key(store):
    _, row = store.mint("sub-alice")
    for _ in range(3):
        store.record_usage(_rec(store, row.key_id, "sub-alice"))
    summary = store.usage(sub="sub-alice")
    assert len(summary) == 1
    s = summary[0]
    assert s["key_id"] == row.key_id
    assert s["requests"] == 3
    assert s["prompt_tokens"] == 39 and s["completion_tokens"] == 36


def test_truncated_requests_are_counted_separately_not_as_zero(store):
    _, row = store.mint("sub-alice")
    store.record_usage(_rec(store, row.key_id, "sub-alice"))
    store.record_usage(_rec(store, row.key_id, "sub-alice", truncated=True,
                            prompt_tokens=None, completion_tokens=None))
    s = store.usage(sub="sub-alice")[0]
    assert s["requests"] == 2
    assert s["truncated_requests"] == 1
    # The known request's tokens are intact; the unknown one added nothing.
    assert s["prompt_tokens"] == 13


def test_usage_is_scoped_to_the_asking_subject(store):
    _, a = store.mint("sub-alice"); _, b = store.mint("sub-bob")
    store.record_usage(_rec(store, a.key_id, "sub-alice"))
    store.record_usage(_rec(store, b.key_id, "sub-bob"))
    assert {r["sub"] for r in store.usage(sub="sub-alice")} == {"sub-alice"}
    assert {r["sub"] for r in store.usage(sub=None)} == {"sub-alice", "sub-bob"}


def test_recording_usage_assigns_a_stable_event_id(store):
    _, row = store.mint("sub-alice")
    store.record_usage(_rec(store, row.key_id, "sub-alice"))
    ids = store.pending_usage_ids()
    assert len(ids) == 1 and len(ids[0]) == 36        # uuid4 string


def test_flush_without_a_registry_is_a_no_op_that_keeps_the_rows(store):
    """dsn=None means nowhere to flush to. The rows must survive, not vanish."""
    _, row = store.mint("sub-alice")
    store.record_usage(_rec(store, row.key_id, "sub-alice"))
    flushed, remaining = store.flush()
    assert flushed == 0 and remaining == 1
    assert len(store.pending_usage_ids()) == 1


# --- the merge rule that a naive refresh gets wrong --------------------------

def test_refresh_can_never_un_revoke_a_locally_revoked_key(store):
    """Revocation is monotonic. If a local revoke has not yet reached the
    registry, pulling the registry's older row back must NOT resurrect the key --
    that would turn a sync into a privilege restoration."""
    full, row = store.mint("sub-alice")
    store.revoke(row.key_id, sub="sub-alice", is_admin=False)
    stale = [{"key_id": row.key_id, "sub": "sub-alice",
              "secret_hash": row.secret_hash, "label": row.label,
              "created_at": row.created_at, "expires_at": None,
              "revoked_at": None, "last_used_at": None}]
    store.apply_registry_rows(stale)
    assert store.authenticate(full, NOW) is None
    assert store.list_keys("sub-alice")[0].revoked_at is not None


def test_refresh_applies_a_revocation_made_elsewhere(store):
    full, row = store.mint("sub-alice")
    remote = [{"key_id": row.key_id, "sub": "sub-alice",
               "secret_hash": row.secret_hash, "label": row.label,
               "created_at": row.created_at, "expires_at": None,
               "revoked_at": "2026-08-19T00:00:00+00:00", "last_used_at": None}]
    store.apply_registry_rows(remote)
    assert store.authenticate(full, NOW) is None


def test_refresh_adds_a_key_minted_elsewhere(store):
    full, key_id, h = keys.generate()
    store.apply_registry_rows([{
        "key_id": key_id, "sub": "sub-bob", "secret_hash": h, "label": "other node",
        "created_at": "2026-08-19T00:00:00+00:00", "expires_at": None,
        "revoked_at": None, "last_used_at": None}])
    assert store.authenticate(full, NOW) is not None


def test_health_reports_the_registry_as_absent_rather_than_lying(store):
    h = store.health()
    assert h["registry_configured"] is False
    assert h["pending_usage"] == 0


def test_expiry_boundary_is_checked_on_both_sides(store):
    full, _ = store.mint("sub-alice", ttl_days=1, now=NOW)
    assert store.authenticate(full, NOW + 86399) is not None   # one second before
    assert store.authenticate(full, NOW + 86400) is None       # exactly at expiry
