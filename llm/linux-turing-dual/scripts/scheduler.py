#!/usr/bin/env python3
"""Which server answers soonest. Pure arithmetic on measured numbers.

THE QUESTION THIS ANSWERS

Not "which server is best" -- that is unanswerable -- but "which of these will
finish this request first, given what they have actually done before". Every
input is either measured by this node or reported by the server itself; nothing
here is a declared specification. A box that claims a 4090 and delivers 40 tok/s
is ranked by the 40.

THE ORDER, AND WHY IT IS THIS ORDER

  1. **Tier**, from an admin-set `priority`. Only the highest tier with a
     candidate is considered. A hard ordering rather than a weight, because an
     operator override whose effect depends on load is not an override.
  2. **Estimated time to finish**, lowest wins:
         queued_ahead x mean_service  +  prompt_tokens / prefill_rate
     Both rates come from this node's own accounting for that server and model.
  3. **Prefix-cache affinity** is a FACTOR on the prefill term, not a veto. The
     same caller returning to the same server was measured here at roughly a
     tenfold saving on prompt processing, so it is worth about that much -- and a
     warm server that is ten times slower still loses, which a veto would get
     wrong.

WHAT AN UNMEASURED SERVER GETS

A conservative default, so it is tried and thereby measured. Zero would rank it
first forever; infinity would mean it never runs again and never acquires a
number. This is the only way a new box can enter the rotation.

THE REASON IS RETURNED, NOT LOGGED

A scheduler that cannot be second-guessed from outside is one nobody can debug,
so `choose` reports which factor decided it and what it expected. Those ride out
on X-Routed-Why and X-Routed-Est.
"""
from __future__ import annotations

from dataclasses import dataclass

# Conservative stand-ins for a server with no history. The prefill figure is well
# below anything measured on this fleet (553 tok/s at 98k on the local pair, 1927
# at 34k), so an unmeasured server looks slower than a known-good one rather than
# more attractive -- it gets tried when nothing better is free, which is enough to
# earn it a real number.
DEFAULT_PREFILL_RATE = 200.0        # tokens/second
DEFAULT_MEAN_SERVICE = 30.0         # seconds per request already in front

# Measured on this project: a warm slot is worth roughly tenfold on prompt
# processing. Expressed as a factor on the prefill term only, because a warm
# cache does nothing for the queue in front of you.
CACHE_HIT_FACTOR = 0.1

# Below this, two estimates are the same answer and the order between them is
# noise. Without it, a rounding difference would flap traffic between two
# equivalent servers and destroy both their caches.
TIE_SECONDS = 0.75


@dataclass
class Candidate:
    """One server that COULD serve this request. Eligibility is decided before
    this module is reached -- nothing here knows about models."""

    server_id: str
    priority: int = 0
    # Requests already accepted and not finished, i.e. what this one waits behind.
    queued_ahead: float = 0.0
    # Measured, or None for "never served anything here yet".
    prefill_rate: float | None = None
    mean_service: float | None = None
    # Does this caller's conversation already live here?
    warm: bool = False
    # False when the server cannot take work right now (no idle pipe). Not an
    # exclusion: it raises the estimate, so a busy server loses to a free one but
    # still beats nothing.
    ready: bool = True


def estimate(c: Candidate, prompt_tokens: float, *,
             ignore_warm: bool = False) -> float:
    """Seconds until this candidate would finish the prefill of this request.

    Completion time is deliberately left out: it depends on how many tokens the
    model chooses to emit, which is unknown at routing time and roughly equal
    across candidates for the same model anyway. Prefill is the part that differs
    by server and by cache state, and at this node's context sizes it dominates --
    177 s of prefill against 1.5 s of generation on a measured 98k request.
    """
    rate = c.prefill_rate or DEFAULT_PREFILL_RATE
    service = c.mean_service if c.mean_service is not None else DEFAULT_MEAN_SERVICE
    prefill = (prompt_tokens or 0) / max(rate, 1e-6)
    if c.warm and not ignore_warm:
        prefill *= CACHE_HIT_FACTOR
    queued = max(0.0, c.queued_ahead)
    if not c.ready:
        # It cannot start until something frees up, which is at least one more
        # service time. Modelled rather than special-cased so it competes on the
        # same scale as everything else.
        queued += 1.0
    return queued * service + prefill


def _tier(candidates: list[Candidate]) -> list[Candidate]:
    top = max(c.priority for c in candidates)
    return [c for c in candidates if c.priority == top]


def _winner(scored: list[tuple]) -> tuple:
    """The lowest estimate, with ties broken by INPUT order.

    The tie band is the point. Two servers whose estimates differ by less than
    TIE_SECONDS are giving the same answer, and picking between them on that
    difference would flap traffic back and forth on rounding noise -- throwing
    away a warm prefix cache each time, which is the one thing this scheduler is
    most careful to preserve. `scored` must therefore arrive in registry order,
    not sorted.
    """
    best = min(e for _, e in scored)
    for pair in scored:
        if pair[1] <= best + TIE_SECONDS:
            return pair
    return scored[0]                    # unreachable: min() is always in band


def rank(candidates: list[Candidate], prompt_tokens: float) -> list[tuple]:
    """[(candidate, estimate)] best first, within the top tier only.

    For inspection and for the panel. Selection uses _winner(), which applies the
    tie band; this view is a plain ordering.
    """
    if not candidates:
        return []
    scored = [(c, estimate(c, prompt_tokens)) for c in _tier(candidates)]
    return sorted(scored, key=lambda pair: pair[1])


def choose(candidates: list[Candidate], prompt_tokens: float = 0.0) -> tuple:
    """(server_id, why, estimate_seconds) or (None, "none-eligible", None).

    `why` names the factor that DECIDED it, which is not always the factor that
    ranked highest:

      only-server  nothing else could have served this
      priority     an operator tier excluded a server that would otherwise have won
      warm         it won because this caller's cache lives there
      fastest      it won on measured speed and current load
    """
    if not candidates:
        return None, "none-eligible", None

    # Scored in REGISTRY order, because _winner breaks ties by position.
    scored = [(c, estimate(c, prompt_tokens)) for c in _tier(candidates)]
    winner, best = _winner(scored)

    if len(candidates) == 1:
        return winner.server_id, "only-server", best

    # Did the tier filter remove a server that would have won? Compared against
    # the full field, so `priority` is claimed only when it actually changed the
    # answer -- otherwise every request on a prioritised fleet would report it and
    # the reason would carry no information.
    full = [(c, estimate(c, prompt_tokens)) for c in candidates]
    if _winner(full)[0].server_id != winner.server_id:
        return winner.server_id, "priority", best

    # Would it still have won without its warm cache? If not, the cache is the
    # reason, and reporting `fastest` would misattribute it to a server that is
    # not.
    others = [e for c, e in scored if c.server_id != winner.server_id]
    if winner.warm and others:
        cold = estimate(winner, prompt_tokens, ignore_warm=True)
        if cold > min(others) + TIE_SECONDS:
            return winner.server_id, "warm", best

    return winner.server_id, "fastest", best
