"""Ranking servers by what they have actually done, not by what they claim.

The rules worth defending, and each has a test that fails if the rule is dropped:

  * an operator tier is absolute, so it cannot be talked out of by load;
  * a warm cache is worth about tenfold and NOT a veto -- a warm server that is
    much slower must lose, which is the case a naive affinity rule gets wrong;
  * an unmeasured server is tried rather than excluded, or it can never acquire
    the measurement that would let it be chosen;
  * the reported reason is the DECIDING factor, because a scheduler nobody can
    second-guess is one nobody can debug.
"""
import pytest

from conftest import load_script

sch = load_script("scheduler")


def C(sid, **kw):
    return sch.Candidate(server_id=sid, **kw)


# --- the estimate -----------------------------------------------------------

def test_a_faster_server_estimates_lower():
    fast = C("fast", prefill_rate=1000.0, mean_service=10.0)
    slow = C("slow", prefill_rate=250.0, mean_service=10.0)
    assert sch.estimate(fast, 10_000) < sch.estimate(slow, 10_000)


def test_a_queue_in_front_of_you_counts():
    idle = C("idle", prefill_rate=500.0, mean_service=20.0, queued_ahead=0)
    busy = C("busy", prefill_rate=500.0, mean_service=20.0, queued_ahead=2)
    assert sch.estimate(busy, 1000) - sch.estimate(idle, 1000) == pytest.approx(40.0)


def test_a_bigger_prompt_costs_more_on_the_same_server():
    s = C("s", prefill_rate=500.0, mean_service=10.0)
    assert sch.estimate(s, 100_000) > sch.estimate(s, 1_000)


def test_a_warm_cache_is_worth_about_tenfold_on_the_prefill_term():
    cold = C("s", prefill_rate=500.0, mean_service=10.0)
    warm = C("s", prefill_rate=500.0, mean_service=10.0, warm=True)
    assert sch.estimate(warm, 50_000) == pytest.approx(
        sch.estimate(cold, 50_000) * sch.CACHE_HIT_FACTOR)


def test_a_warm_cache_does_nothing_for_the_queue_in_front_of_you():
    """It saves prompt processing, not waiting. Applying it to the whole estimate
    would make a busy warm server look free."""
    warm_busy = C("s", prefill_rate=500.0, mean_service=20.0, queued_ahead=3, warm=True)
    assert sch.estimate(warm_busy, 1000) > 3 * 20.0


def test_a_server_that_cannot_start_now_is_penalised_not_excluded():
    free = C("free", prefill_rate=500.0, mean_service=20.0)
    blocked = C("blocked", prefill_rate=500.0, mean_service=20.0, ready=False)
    assert sch.estimate(blocked, 1000) > sch.estimate(free, 1000)
    assert sch.choose([blocked], 1000)[0] == "blocked"     # still usable alone


def test_an_unmeasured_server_uses_a_conservative_default():
    unknown = C("new")
    known = C("known", prefill_rate=sch.DEFAULT_PREFILL_RATE * 3, mean_service=5.0)
    assert sch.estimate(unknown, 10_000) > sch.estimate(known, 10_000)
    # But it is still a candidate, which is the only way it gets measured.
    assert sch.choose([unknown], 10_000)[0] == "new"


def test_a_zero_rate_does_not_divide_by_zero():
    assert sch.estimate(C("s", prefill_rate=0.0), 1000) > 0


# --- choosing ---------------------------------------------------------------

def test_nothing_eligible_is_reported_as_such():
    assert sch.choose([], 100) == (None, "none-eligible", None)


def test_one_candidate_says_so_rather_than_claiming_it_was_fastest():
    sid, why, est = sch.choose([C("only", prefill_rate=500.0)], 1000)
    assert (sid, why) == ("only", "only-server")
    assert est > 0


def test_the_fastest_wins_when_nothing_else_distinguishes_them():
    sid, why, _ = sch.choose([C("slow", prefill_rate=100.0, mean_service=10.0),
                              C("fast", prefill_rate=2000.0, mean_service=10.0)],
                             50_000)
    assert (sid, why) == ("fast", "fastest")


def test_load_can_beat_raw_speed():
    """The point of measuring load at all: a fast box with four requests queued
    is the wrong answer."""
    sid, _, _ = sch.choose([C("fast", prefill_rate=2000.0, mean_service=30.0,
                              queued_ahead=4),
                            C("idle", prefill_rate=500.0, mean_service=30.0)],
                           10_000)
    assert sid == "idle"


def test_an_operator_tier_is_absolute():
    """It must not be talked out of by load, or it is not an override."""
    sid, why, _ = sch.choose([C("preferred", priority=1, prefill_rate=100.0,
                                mean_service=60.0, queued_ahead=3),
                              C("quick", prefill_rate=5000.0, mean_service=1.0)],
                             10_000)
    assert (sid, why) == ("preferred", "priority")


def test_a_tier_is_only_reported_when_it_changed_the_answer():
    """Otherwise every request on a prioritised fleet would claim `priority` and
    the reason would carry no information."""
    sid, why, _ = sch.choose([C("a", priority=1, prefill_rate=5000.0, mean_service=1.0),
                              C("b", prefill_rate=100.0, mean_service=60.0)],
                             10_000)
    assert (sid, why) == ("a", "fastest")


def test_a_lower_tier_is_used_when_no_higher_one_is_a_candidate():
    sid, why, _ = sch.choose([C("last-resort", priority=-5, prefill_rate=500.0)], 1000)
    assert (sid, why) == ("last-resort", "only-server")


def test_a_warm_server_wins_and_says_why():
    sid, why, _ = sch.choose([C("warm", prefill_rate=500.0, mean_service=10.0,
                                warm=True),
                              C("cold", prefill_rate=600.0, mean_service=10.0)],
                             50_000)
    assert (sid, why) == ("warm", "warm")


def test_a_warm_server_that_is_much_slower_still_loses():
    """The case a naive affinity rule gets wrong. Ten times the cache saving does
    not buy a hundred times the slowness."""
    sid, _, _ = sch.choose([C("warm-but-crawling", prefill_rate=20.0,
                              mean_service=10.0, warm=True),
                            C("cold-and-quick", prefill_rate=5000.0,
                              mean_service=10.0)],
                           50_000)
    assert sid == "cold-and-quick"


def test_warm_is_not_claimed_when_it_would_have_won_anyway():
    """`warm` must mean the cache decided it. Saying so about a server that was
    already fastest would misattribute the reason."""
    sid, why, _ = sch.choose([C("warm", prefill_rate=5000.0, mean_service=1.0,
                                warm=True),
                              C("cold", prefill_rate=50.0, mean_service=60.0)],
                             50_000)
    assert (sid, why) == ("warm", "fastest")


def test_an_estimate_is_returned_so_the_decision_can_be_checked_from_outside():
    sid, why, est = sch.choose([C("a", prefill_rate=1000.0, mean_service=10.0,
                                  queued_ahead=1),
                                C("b", prefill_rate=1000.0, mean_service=10.0,
                                  queued_ahead=2)], 10_000)
    assert sid == "a"
    assert est == pytest.approx(10.0 + 10.0)


def test_ties_keep_registry_order_rather_than_flapping():
    """Two equivalent servers must not swap on rounding noise: each swap throws
    away a warm cache."""
    twins = [C("first", prefill_rate=500.0, mean_service=10.0),
             C("second", prefill_rate=500.0000001, mean_service=10.0)]
    assert sch.choose(twins, 10_000)[0] == "first"
    assert sch.choose(list(reversed(twins)), 10_000)[0] == "second"
