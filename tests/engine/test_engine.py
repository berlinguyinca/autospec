"""Tests for the NORMAL/COMPACTED/ROLLED state machine engine."""

import pytest

from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Action, Engine, State


def u(pct: float, max_tokens: int = 10_000) -> Usage:
    """Helper: build a Usage at *pct* (0.0–1.0)."""
    return Usage(
        used_tokens=int(pct * max_tokens),
        max_tokens=max_tokens,
        model="test-model",
        estimated=False,
    )


# ---------------------------------------------------------------------------
# Basic single-transition tests
# ---------------------------------------------------------------------------

def test_normal_to_compacted_at_fifty():
    e = Engine()
    actions = e.classify(u(0.50))
    assert [a.kind for a in actions] == ["compact"]
    assert e.state is State.COMPACTED


def test_compacted_to_normal_when_below_thirty():
    e = Engine()
    e.classify(u(0.50))  # NORMAL → COMPACTED
    actions = e.classify(u(0.25))  # COMPACTED → NORMAL
    assert [a.kind for a in actions] == ["noop"]
    assert actions[0].payload == "reset:compacted->normal"
    assert e.state is State.NORMAL


def test_compacted_to_rolled_at_eighty():
    e = Engine()
    e.classify(u(0.50))  # NORMAL → COMPACTED
    actions = e.classify(u(0.80))  # COMPACTED → ROLLED
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"]
    assert e.state is State.ROLLED


def test_normal_direct_to_rolled_at_eighty():
    """Spec: NORMAL → ROLLED direct climb (skips COMPACTED)."""
    e = Engine()
    actions = e.classify(u(0.80))
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"]
    assert e.state is State.ROLLED


# ---------------------------------------------------------------------------
# Anti-flapping invariant
# ---------------------------------------------------------------------------

def test_no_back_to_back_compact_without_reset():
    """Sequence [10%, 52%, 55%, 58%] fires compact exactly once."""
    e = Engine()
    compact_count = 0
    for pct in [0.10, 0.52, 0.55, 0.58]:
        actions = e.classify(u(pct))
        compact_count += sum(1 for a in actions if a.kind == "compact")
    assert compact_count == 1


def test_noop_in_middle_of_band_normal():
    """Between 30% and 50% in NORMAL state — no action."""
    e = Engine()
    actions = e.classify(u(0.40))
    assert actions == []
    assert e.state is State.NORMAL


def test_noop_between_thirty_and_eighty_in_compacted():
    """Between 30% and 80% while COMPACTED — no action (anti-flap hold)."""
    e = Engine()
    e.classify(u(0.50))  # → COMPACTED
    actions = e.classify(u(0.55))
    assert actions == []
    assert e.state is State.COMPACTED


# ---------------------------------------------------------------------------
# Full scripted sequence
# ---------------------------------------------------------------------------

def test_full_scripted_sequence_10_30_51_25_49_52_75_81():
    """Spec example: 10→30→51→25→49→52→75→81."""
    e = Engine()

    results = []
    for pct in [0.10, 0.30, 0.51, 0.25, 0.49, 0.52, 0.75, 0.81]:
        results.append((pct, [a.kind for a in e.classify(u(pct))]))

    # 10%: NORMAL, below threshold → noop
    assert results[0] == (0.10, [])
    # 30%: NORMAL, between 30-50 → noop
    assert results[1] == (0.30, [])
    # 51%: NORMAL >= 50% → compact
    assert results[2] == (0.51, ["compact"])
    # 25%: COMPACTED < 30% → noop(reset)
    assert results[3] == (0.25, ["noop"])
    # 49%: NORMAL, below 50% → noop
    assert results[4] == (0.49, [])
    # 52%: NORMAL >= 50% → compact again (state reset after drop)
    assert results[5] == (0.52, ["compact"])
    # 75%: COMPACTED, between 30-80 → noop
    assert results[6] == (0.75, [])
    # 81%: COMPACTED >= 80% → handoff, clear, resume
    assert results[7] == (0.81, ["handoff", "clear", "resume"])


# ---------------------------------------------------------------------------
# ROLLED state reset
# ---------------------------------------------------------------------------

def test_rolled_to_normal_when_below_thirty():
    e = Engine()
    e.classify(u(0.80))  # NORMAL → ROLLED
    actions = e.classify(u(0.20))  # ROLLED → NORMAL
    assert [a.kind for a in actions] == ["noop"]
    assert actions[0].payload == "reset:rolled->normal"
    assert e.state is State.NORMAL


def test_rolled_noop_above_thirty():
    """ROLLED stays ROLLED if pct is still >= 30%."""
    e = Engine()
    e.classify(u(0.80))  # → ROLLED
    actions = e.classify(u(0.50))
    assert actions == []
    assert e.state is State.ROLLED


def test_second_rollover_after_reset():
    """After ROLLED→NORMAL reset, the engine can roll again."""
    e = Engine()
    e.classify(u(0.80))  # → ROLLED
    e.classify(u(0.20))  # → NORMAL
    actions = e.classify(u(0.90))  # → ROLLED again
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"]
    assert e.state is State.ROLLED


# ---------------------------------------------------------------------------
# Action dataclass
# ---------------------------------------------------------------------------

def test_action_defaults():
    a = Action("compact")
    assert a.kind == "compact"
    assert a.payload == ""


def test_action_with_payload():
    a = Action("noop", "reset:rolled->normal")
    assert a.payload == "reset:rolled->normal"
