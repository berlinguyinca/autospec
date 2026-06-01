"""State-machine test: COMPACTED stays COMPACTED after rollover cancel.

Scenario: Engine is in COMPACTED state. Usage climbs to 80%. Engine emits
[handoff, clear, resume]. The cancel window fires for 'clear' and returns True.
The caller reverts engine state to COMPACTED. On the next 80% reading the
engine re-fires the rollover sequence (handoff, clear, resume again).
"""
from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Engine, State


def _usage(pct: float) -> Usage:
    return Usage(used_tokens=int(pct * 1000), max_tokens=1000, model="test", estimated=False)


def test_compacted_stays_compacted_after_cancel():
    """Engine re-fires rollover when state is reverted to COMPACTED after cancel."""
    engine = Engine()

    # Climb to COMPACTED (50%)
    actions = engine.classify(_usage(0.50))
    assert engine.state is State.COMPACTED
    assert any(a.kind == "compact" for a in actions)

    # Climb to 80% → engine emits rollover sequence and moves to ROLLED
    actions = engine.classify(_usage(0.80))
    assert engine.state is State.ROLLED
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"]

    # Simulate: cancel window fired for 'clear' → caller reverts to COMPACTED
    engine._state = State.COMPACTED  # noqa: SLF001

    assert engine.state is State.COMPACTED

    # Next 80% tick: engine should re-fire the full rollover sequence
    actions = engine.classify(_usage(0.80))
    assert engine.state is State.ROLLED
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"], (
        "After cancel+revert, the next 80% reading must re-trigger rollover"
    )
