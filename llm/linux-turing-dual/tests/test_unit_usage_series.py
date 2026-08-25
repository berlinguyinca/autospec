"""Usage over time, and sessions derived from it.

Two claims in this data are easy to state wrongly:

  * "how many people used the node" is DISTINCT identities in a window. Summing
    a per-day user count across days answers a different question and inflates
    it -- somebody active on ten days is one person, not ten.
  * a "session" is not recorded anywhere. It is a convention: a run of requests
    by one identity with no gap longer than N minutes. It must be reported as
    derived, and it must split on the gap rather than on the calendar.
"""
import datetime as dt
import time

from nodescripts import load_script

ks = load_script("keystore")


def iso(epoch):
    return dt.datetime.fromtimestamp(epoch, dt.timezone.utc).isoformat()


def store(tmp_path):
    s = ks.KeyStore(str(tmp_path / "k.sqlite3"))
    s.migrate_local()
    return s


def ev(s, sub, at, key="k1", model="m", prompt=10, completion=5, status=200):
    s.record_usage({"ts": at, "sub": sub, "key_id": key, "model": model,
                    "prompt_tokens": prompt, "completion_tokens": completion,
                    "status_code": status, "endpoint": "/v1/chat/completions"})


# --- series -----------------------------------------------------------------

def test_requests_and_tokens_bucket_by_day(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 3600)
    ev(s, "alice", now - 3500)
    ev(s, "bob", now - 3400)
    rows = s.usage_series(days=2, bucket="day")
    assert rows, "no buckets returned"
    total = sum(r["requests"] for r in rows)
    assert total == 3
    assert sum(r["total_tokens"] for r in rows) == 45


def test_users_per_bucket_is_distinct_not_a_request_count(tmp_path):
    s = store(tmp_path)
    now = time.time()
    for _ in range(6):
        ev(s, "alice", now - 600)
    ev(s, "bob", now - 600)
    rows = s.usage_series(days=1, bucket="hour")
    assert sum(r["requests"] for r in rows) == 7
    assert max(r["users"] for r in rows) == 2, "users must be DISTINCT identities"


def test_errors_are_counted_separately(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 60, status=200)
    ev(s, "alice", now - 50, status=503)
    ev(s, "alice", now - 40, status=429)
    rows = s.usage_series(days=1, bucket="hour")
    assert sum(r["errors"] for r in rows) == 2


def test_the_window_actually_excludes_older_events(tmp_path):
    """usage() accepted a `days` argument and ignored it. This one must not."""
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 40 * 86400)
    ev(s, "alice", now - 3600)
    assert sum(r["requests"] for r in s.usage_series(days=2)) == 1
    assert sum(r["requests"] for r in s.usage_series(days=60)) == 2


def test_scoping_to_one_identity(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 60)
    ev(s, "bob", now - 60)
    assert sum(r["requests"] for r in s.usage_series(days=1, sub="alice")) == 1


# --- sessions ---------------------------------------------------------------

def test_a_run_of_requests_is_one_session(tmp_path):
    s = store(tmp_path)
    now = time.time()
    for i in range(5):
        ev(s, "alice", now - 600 + i * 60)
    out = s.usage_sessions(days=1)
    assert len(out) == 1
    assert out[0]["requests"] == 5


def test_a_long_gap_splits_the_session(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 7200)
    ev(s, "alice", now - 60)          # ~2h later, well past the 30 min gap
    assert len(s.usage_sessions(days=1, gap_minutes=30)) == 2


def test_sessions_do_not_span_two_people(tmp_path):
    """Interleaved traffic must not be sewn into one session."""
    s = store(tmp_path)
    now = time.time()
    for i in range(4):
        ev(s, "alice", now - 600 + i * 60)
        ev(s, "bob", now - 590 + i * 60)
    out = s.usage_sessions(days=1)
    assert len(out) == 2
    assert {o["sub"] for o in out} == {"alice", "bob"}


def test_a_session_reports_span_and_distinct_keys(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 600, key="k1")
    ev(s, "alice", now - 300, key="k2")
    o = s.usage_sessions(days=1)[0]
    assert o["keys"] == 2
    assert 250 <= o["seconds"] <= 350
    assert o["ended"] >= o["started"]


def test_newest_session_first(tmp_path):
    s = store(tmp_path)
    now = time.time()
    ev(s, "alice", now - 100000)
    ev(s, "bob", now - 60)
    out = s.usage_sessions(days=30)
    assert out[0]["sub"] == "bob"
