"""Banning an address that keeps presenting credentials that do not work.

Two things here are easy to get wrong and expensive to get wrong, so most of
this file is about them rather than about the counting:

  * IDENTITY. Behind nginx every request arrives from 127.0.0.1. Ban that and
    you have banned everyone. Take the client's word for who they are and a
    caller picks their own identity -- rotating past a ban, or framing someone
    else into one.
  * WHAT COUNTS. This node ran for months with an internal component polling
    with a stale credential ~2x/sec. A lockout that counted loopback would have
    banned the node from itself within seconds of being installed.
"""
import time

from nodescripts import load_script

lk = load_script("lockout")


def setup_function():
    lk.reset_for_tests()


def hdr(**kw):
    return lambda name: kw.get(name.replace("-", "_"))


# --- counting ---------------------------------------------------------------

def test_under_the_threshold_is_not_a_ban():
    t = 1000.0
    for i in range(lk.FAILS - 1):
        assert lk.record_failure("203.0.113.9", t + i) is None
    assert lk.banned_until("203.0.113.9", t + 10) is None


def test_the_threshold_bans():
    t = 1000.0
    until = None
    for i in range(lk.FAILS):
        until = lk.record_failure("203.0.113.9", t + i)
    assert until is not None
    assert lk.banned_until("203.0.113.9", t + 10) == until


def test_failures_outside_the_window_do_not_accumulate():
    """Otherwise a wrong key once a week eventually bans a legitimate user."""
    ip = "203.0.113.10"
    for i in range(lk.FAILS - 1):
        lk.record_failure(ip, 1000.0 + i)
    # long after the window
    assert lk.record_failure(ip, 1000.0 + lk.WINDOW + 60) is None
    assert lk.banned_until(ip, 1000.0 + lk.WINDOW + 61) is None


def test_a_ban_expires_on_its_own():
    ip = "203.0.113.11"
    t = 1000.0
    for i in range(lk.FAILS):
        until = lk.record_failure(ip, t + i)
    assert lk.banned_until(ip, until - 1) is not None
    assert lk.banned_until(ip, until + 1) is None


def test_clear_lifts_a_ban():
    ip = "203.0.113.12"
    for i in range(lk.FAILS):
        lk.record_failure(ip, 1000.0 + i)
    assert lk.banned_until(ip, 1001.0) is not None
    lk.clear(ip)
    assert lk.banned_until(ip, 1001.0) is None


# --- loopback is never the enemy --------------------------------------------

def test_loopback_is_never_counted_or_banned():
    for ip in ("127.0.0.1", "::1"):
        for i in range(lk.FAILS * 3):
            assert lk.record_failure(ip, 1000.0 + i) is None
        assert lk.banned_until(ip, 2000.0) is None


# --- identity ---------------------------------------------------------------

def test_x_real_ip_is_trusted_from_the_proxy():
    assert lk.client_ip(hdr(X_Real_IP="198.51.100.7"), "127.0.0.1") == "198.51.100.7"


def test_x_real_ip_is_ignored_from_a_stranger():
    """A direct caller must not be able to name itself something else -- that is
    both a ban evasion and a way to get another address banned."""
    got = lk.client_ip(hdr(X_Real_IP="198.51.100.7"), "203.0.113.50")
    assert got == "203.0.113.50"


def test_no_header_falls_back_to_the_peer():
    assert lk.client_ip(hdr(), "203.0.113.51") == "203.0.113.51"


def test_a_blank_header_falls_back_to_the_peer():
    assert lk.client_ip(hdr(X_Real_IP="   "), "127.0.0.1") == "127.0.0.1"


# --- persistence ------------------------------------------------------------

def test_a_ban_survives_a_restart(tmp_path):
    db = str(tmp_path / "keys.sqlite3")
    lk.configure(db)
    ip = "203.0.113.99"
    now = time.time()
    for i in range(lk.FAILS):
        until = lk.record_failure(ip, now + i)
    assert until is not None

    # a "restart": drop all in-memory state, then reload from the same db
    lk.reset_for_tests()
    assert lk.banned_until(ip, now + 1) is None, "precondition: memory cleared"
    lk.configure(db)
    assert lk.banned_until(ip, now + 1) is not None, "ban did not survive"


def test_an_unusable_database_does_not_stop_the_gateway():
    """Persistence is hardening, not a precondition. Refusing to start because a
    ban table is unavailable trades a small security property for an outage."""
    lk.configure("/nonexistent-dir/nope.sqlite3")
    assert lk.record_failure("203.0.113.77", 1000.0) is None   # no exception


# --- bounded ----------------------------------------------------------------
# A counter map keyed by address, with no ceiling, is a memory-exhaustion vector
# wearing the costume of a security control -- and nothing rotates source
# addresses faster than the thing this module exists to stop.

def test_rotating_sources_do_not_grow_the_counter_map_without_bound():
    lk.MAX_TRACKED = 500          # keep the test quick; the mechanism is the point
    now = 1000.0
    for i in range(5000):
        lk.record_failure(f"198.51.{i // 256 % 256}.{i % 256}", now)
    assert len(lk._FAILS) <= lk.MAX_TRACKED + lk.SWEEP_EVERY, len(lk._FAILS)


def test_a_sweep_never_forgets_a_live_ban():
    """Shedding counters is fine; shedding a ban would undo the whole point."""
    lk.MAX_TRACKED = 100
    now = 1000.0
    for i in range(lk.FAILS):
        until = lk.record_failure("203.0.113.200", now + i)
    assert until is not None
    for i in range(3000):
        lk.record_failure(f"198.51.{i // 256 % 256}.{i % 256}", now + 10)
    assert lk.banned_until("203.0.113.200", now + 20) is not None


def test_stale_counters_are_dropped_once_their_window_has_passed():
    lk.MAX_TRACKED = 20000
    now = 1000.0
    for i in range(600):          # enough to cross SWEEP_EVERY
        lk.record_failure(f"203.0.114.{i % 256}", now)
    before = len(lk._FAILS)
    for i in range(600):          # long after the first window closed
        lk.record_failure(f"203.0.115.{i % 256}", now + lk.WINDOW + 60)
    assert len(lk._FAILS) < before + 600, "old windows were never dropped"
