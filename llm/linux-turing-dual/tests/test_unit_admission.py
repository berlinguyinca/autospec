"""Admission control: holding a request beats killing every live session.

A context tier here has always been a CONTRACT and never a limit. `qwen3.5-9b` is
81920 tokens across 2 slots; `qwen3.5-9b-80k` claims the whole pool by name. The
two together ask for 122880 of 81920, and llama.cpp does not refuse — it dies,
taking every live session with it.

These tests run REAL THREADS against the real condition variable, because the
properties that matter are all about what happens while something else is
waiting. A single-threaded test of a queue proves only the empty case.
"""
import threading
import time

from nodescripts import load_script

adm = load_script("admission")


def _bg(fn):
    t = threading.Thread(target=fn, daemon=True)
    t.start()
    return t


# --- what a request is claiming ---------------------------------------------

def test_a_named_tier_is_taken_at_its_word():
    assert adm.tier_tokens("qwen3.5-9b-80k", 81920, 2) == 80 * 1024
    assert adm.tier_tokens("qwen3.8-27b-100k", 102400, 1) == 100 * 1024
    assert adm.tier_tokens("qwen3.8-27b-40k", 102400, 2) == 40 * 1024


def test_an_untiered_id_gets_the_fair_share_the_preset_configured():
    """Not the whole pool -- that would serialise ordinary traffic -- and not a
    token, which would defeat the control. `context / slots` is what "two seats"
    already meant."""
    assert adm.tier_tokens("qwen3.8-27b", 102400, 2) == 51200
    assert adm.tier_tokens("qwen3.5-9b", 81920, 2) == 40960
    assert adm.tier_tokens("qwen3.8-27b-100k", 102400, 1) == 102400


def test_a_tier_inside_a_longer_id_is_still_found():
    assert adm.tier_tokens("qwen3.8-27b-vision-40k", 81920, 2) == 40 * 1024


def test_an_unknown_shape_claims_nothing_and_says_so():
    """0 lets the caller decide. Inventing a number here would be a guess wearing
    the authority of a measurement."""
    assert adm.tier_tokens("mystery-model", 0, 0) == 0


# --- the documented crash ---------------------------------------------------

def test_the_pair_that_used_to_kill_the_node_now_queues():
    """80k + 40k against an 81920 pool. Both used to die together."""
    pool = adm.Pool(81920)
    big = pool.acquire(80 * 1024, timeout=1)
    assert big is not None

    small = {}
    def take():
        small["lease"] = pool.acquire(40 * 1024, timeout=5)
    t = _bg(take)
    time.sleep(0.2)
    assert small.get("lease") is None, "the second session must WAIT, not proceed"
    assert pool.waiting == 1

    pool.release(big)
    t.join(timeout=5)
    assert small["lease"] is not None, "it must be admitted once the pool frees"
    assert small["lease"].waited > 0


def test_two_sessions_that_fit_run_at_once():
    """The hold is only meaningful if what fits is not delayed."""
    pool = adm.Pool(81920)
    a = pool.acquire(40 * 1024, timeout=1)
    b = pool.acquire(40 * 1024, timeout=1)
    assert a is not None and b is not None
    assert pool.waiting == 0
    assert b.waited < 0.5


# --- fairness ---------------------------------------------------------------

def test_a_large_request_is_not_starved_by_a_stream_of_small_ones():
    """Admitting whoever happens to fit starves the big session forever -- and
    that is the caller most likely to be a person waiting."""
    pool = adm.Pool(100)
    held = pool.acquire(60, timeout=1)          # pool: 60/100 used
    big = {}
    def take_big():
        big["lease"] = pool.acquire(100, timeout=6)   # needs the whole pool
    t = _bg(take_big)
    time.sleep(0.2)                              # let it reach the head

    # A parade of small requests arrives while the big one waits. Every one of
    # them would fit in the 40 free tokens.
    blocked = []
    for _ in range(5):
        blocked.append(pool.acquire(10, timeout=0.3))
    assert all(x is None for x in blocked), "small requests jumped the queue"

    pool.release(held)
    t.join(timeout=6)
    assert big["lease"] is not None


def test_order_is_preserved_among_waiters():
    pool = adm.Pool(100)
    held = pool.acquire(100, timeout=1)
    order = []
    def waiter(n):
        def go():
            lease = pool.acquire(50, timeout=6)
            if lease:
                order.append(n)
        return go
    threads = []
    for n in (1, 2):
        threads.append(_bg(waiter(n)))
        time.sleep(0.15)                         # establish arrival order
    pool.release(held)
    for t in threads:
        t.join(timeout=6)
    assert order == [1, 2]


# --- refusing what can never fit -------------------------------------------

def test_a_request_bigger_than_the_pool_is_refused_at_once():
    """Waiting for capacity that cannot exist is a hang dressed up as fairness."""
    pool = adm.Pool(81920)
    started = time.monotonic()
    try:
        pool.acquire(100 * 1024, timeout=30)
        raise AssertionError("should have refused")
    except adm.TooLarge as exc:
        assert "exceeds" in str(exc)
    assert time.monotonic() - started < 1, "it must not have waited"
    assert pool.snapshot()["rejected"] == 1


def test_a_request_exactly_the_size_of_the_pool_is_allowed():
    """The boundary belongs on the allowed side: `-100k` against a 102400 pool is
    the configuration working as designed, not an error."""
    pool = adm.Pool(102400)
    assert pool.acquire(100 * 1024, timeout=1) is not None


# --- giving up ------------------------------------------------------------

def test_a_waiter_that_times_out_returns_rather_than_hangs():
    pool = adm.Pool(100)
    pool.acquire(100, timeout=1)
    started = time.monotonic()
    assert pool.acquire(50, timeout=0.4) is None
    assert 0.3 < time.monotonic() - started < 3


def test_a_caller_that_went_away_stops_holding_its_place():
    """Otherwise the queue fills with ghosts and real callers wait behind them."""
    pool = adm.Pool(100)
    held = pool.acquire(100, timeout=1)
    gone = threading.Event()
    result = {}
    def go():
        result["lease"] = pool.acquire(50, timeout=10,
                                       cancelled=lambda: gone.is_set())
    t = _bg(go)
    time.sleep(0.2)
    assert pool.waiting == 1
    gone.set()
    t.join(timeout=5)
    assert result["lease"] is None
    assert pool.waiting == 0, "the abandoned waiter must leave the queue"
    pool.release(held)


def test_a_timed_out_waiter_does_not_block_the_one_behind_it():
    pool = adm.Pool(100)
    held = pool.acquire(100, timeout=1)
    got = {}
    def impatient():
        got["a"] = pool.acquire(100, timeout=0.3)
    def patient():
        got["b"] = pool.acquire(50, timeout=6)
    ta = _bg(impatient)
    time.sleep(0.1)
    tb = _bg(patient)
    ta.join(timeout=4)
    pool.release(held)
    tb.join(timeout=6)
    assert got["a"] is None and got["b"] is not None


# --- accounting -----------------------------------------------------------

def test_releasing_returns_the_capacity()  :
    pool = adm.Pool(100)
    lease = pool.acquire(60, timeout=1)
    assert pool.used == 60 and pool.free == 40
    pool.release(lease)
    assert pool.used == 0 and pool.free == 100


def test_releasing_nothing_is_harmless():
    """The relay's finally-block releases whatever it has, which may be nothing
    when admission itself failed."""
    pool = adm.Pool(100)
    pool.release(None)
    assert pool.used == 0


def test_capacity_cannot_be_released_twice_into_existence():
    pool = adm.Pool(100)
    lease = pool.acquire(60, timeout=1)
    pool.release(lease)
    pool.release(lease)
    assert pool.used == 0, "a double release must not manufacture capacity"


def test_a_hundred_concurrent_requests_never_exceed_the_pool():
    """The property that matters: whatever the interleaving, the sum of what is
    admitted never exceeds what exists."""
    pool = adm.Pool(1000)
    peak = {"max": 0}
    lock = threading.Lock()
    def go():
        lease = pool.acquire(100, timeout=15)
        if lease is None:
            return
        with lock:
            peak["max"] = max(peak["max"], pool.used)
        time.sleep(0.01)
        pool.release(lease)
    threads = [_bg(go) for _ in range(100)]
    for t in threads:
        t.join(timeout=20)
    assert peak["max"] <= 1000
    assert pool.used == 0, "every lease must have been returned"


# --- pools are per loaded model ------------------------------------------

def test_each_model_on_each_upstream_gets_its_own_pool():
    a = adm.Admission()
    p1 = a.pool("local", "qwen3.8-27b", 102400)
    p2 = a.pool("local", "qwen3.5-9b", 81920)
    p3 = a.pool("bender", "qwen3.8-27b", 102400)
    assert p1 is not p2 and p1 is not p3
    assert a.pool("local", "qwen3.8-27b", 102400) is p1


def test_a_reloaded_model_with_a_new_context_gets_a_new_pool():
    """Outstanding leases belong to the old allocation and must drain against it,
    so the pool is replaced rather than resized under them."""
    a = adm.Admission()
    p1 = a.pool("local", "m", 1000)
    p1.acquire(500, timeout=1)
    p2 = a.pool("local", "m", 2000)
    assert p2 is not p1 and p2.used == 0 and p1.used == 500
