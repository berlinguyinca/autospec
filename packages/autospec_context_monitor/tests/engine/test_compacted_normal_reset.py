"""Regression tests for g-009b: COMPACTED → NORMAL reset with anti-flapping coverage.

Spec §Threshold state machine:
- COMPACTED → NORMAL when pct < 0.30 (compaction worked)
- Must NOT re-enter NORMAL on same tick it entered COMPACTED
- Must NOT reset at pct >= 0.30 (anti-flapping)
"""
from __future__ import annotations

from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Engine, State


def _usage(pct: float) -> Usage:
    return Usage(used_tokens=int(pct * 1000), max_tokens=1000, model="test", estimated=False)


def _tick_to_compacted(engine: Engine) -> None:
    """Drive engine to COMPACTED state via a 65% tick."""
    engine.classify(_usage(0.65))
    assert engine.state is State.COMPACTED


def test_compacted_resets_to_normal_below_30pct() -> None:
    """COMPACTED → NORMAL transition fires when pct drops below 0.30."""
    e = Engine()
    _tick_to_compacted(e)
    actions = e.classify(_usage(0.29))
    assert e.state is State.NORMAL
    # Engine emits a noop action on this transition to record the event.
    assert len(actions) == 1
    assert actions[0].kind == "noop"


def test_compacted_no_reset_at_31pct() -> None:
    """COMPACTED does not reset at pct=0.31 (anti-flapping: threshold is strictly < 0.30)."""
    e = Engine()
    _tick_to_compacted(e)
    e.classify(_usage(0.31))
    assert e.state is State.COMPACTED


def test_compacted_to_normal_emits_no_compact_or_rollover_actions() -> None:
    """On COMPACTED → NORMAL reset tick, no compact/handoff/clear/resume actions are emitted."""
    e = Engine()
    _tick_to_compacted(e)
    actions = e.classify(_usage(0.20))
    assert e.state is State.NORMAL
    action_kinds = [a.kind for a in actions]
    assert "compact" not in action_kinds
    assert "handoff" not in action_kinds
    assert "clear" not in action_kinds
    assert "resume" not in action_kinds
