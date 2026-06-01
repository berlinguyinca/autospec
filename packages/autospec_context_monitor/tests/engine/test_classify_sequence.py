"""Engine scripted-sequence test — spec §Testing strategy Layer 2.

Drives Engine.classify() with the exact 8-step percent sequence from the spec:
  [10%, 30%, 51%, 25%, 49%, 52%, 75%, 81%]

and asserts the resulting state transitions and action streams.
"""
from __future__ import annotations

import pytest

from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Engine, State


def _usage(pct: float) -> Usage:
    return Usage(used_tokens=int(pct * 1000), max_tokens=1000, model="test", estimated=False)


# Spec sequence from §Testing strategy Layer 2
SEQUENCE = [0.10, 0.30, 0.51, 0.25, 0.49, 0.52, 0.75, 0.81]

# Expected state after each tick
EXPECTED_STATES = [
    State.NORMAL,     # 10% — below compact threshold
    State.NORMAL,     # 30% — below compact threshold
    State.COMPACTED,  # 51% — triggers compact
    State.NORMAL,     # 25% — compaction worked; reset to NORMAL
    State.NORMAL,     # 49% — below compact threshold
    State.COMPACTED,  # 52% — triggers compact again
    State.COMPACTED,  # 75% — anti-flap: stays COMPACTED (below 80%, above 30%)
    State.ROLLED,     # 81% — triggers handoff/clear/resume
]

# Expected action kind-lists per tick
# Note: COMPACTED→NORMAL reset emits a noop action ("noop") per engine design.
EXPECTED_ACTION_KINDS = [
    [],                              # 10%
    [],                              # 30%
    ["compact"],                     # 51%
    ["noop"],                        # 25% — reset:compacted->normal
    [],                              # 49%
    ["compact"],                     # 52%
    [],                              # 75% — anti-flap hold
    ["handoff", "clear", "resume"],  # 81%
]


def test_classify_full_scripted_sequence():
    """8-step scripted sequence matches expected states and action streams."""
    engine = Engine()
    actual_states: list[State] = []
    actual_actions: list[list[str]] = []

    for pct in SEQUENCE:
        result = engine.classify(_usage(pct))
        actual_states.append(engine.state)
        actual_actions.append([a.kind for a in result])

    assert actual_states == EXPECTED_STATES, (
        f"State mismatch.\n  expected: {EXPECTED_STATES}\n  actual:   {actual_states}"
    )
    assert actual_actions == EXPECTED_ACTION_KINDS, (
        f"Action mismatch.\n  expected: {EXPECTED_ACTION_KINDS}\n  actual:   {actual_actions}"
    )
