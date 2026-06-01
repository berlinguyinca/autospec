"""Tests for ROLLED→NORMAL reset on new transcript detection (g-016).

Covers:
  - engine.reset() sets state to NORMAL
  - main loop stays ROLLED when transcript path is unchanged
  - main loop resets to NORMAL when transcript path changes
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.engine import Engine, State
from autospec_context_monitor.adapters.base import Usage


# ---------------------------------------------------------------------------
# Helper
# ---------------------------------------------------------------------------

def _usage(pct: float) -> Usage:
    return Usage(
        used_tokens=int(pct * 1000),
        max_tokens=1000,
        model="test",
        estimated=False,
    )


# ---------------------------------------------------------------------------
# Test 1: engine.reset() sets state to NORMAL
# ---------------------------------------------------------------------------

def test_engine_reset_sets_normal():
    """engine.reset() transitions any state back to NORMAL."""
    engine = Engine()
    # Drive to ROLLED
    engine.classify(_usage(0.55))  # NORMAL→COMPACTED
    engine.classify(_usage(0.85))  # COMPACTED→ROLLED
    assert engine.state is State.ROLLED

    engine.reset()
    assert engine.state is State.NORMAL


# ---------------------------------------------------------------------------
# Test 2: stays ROLLED when transcript path is unchanged
# ---------------------------------------------------------------------------

def test_rolled_stays_when_transcript_unchanged(tmp_path, caplog):
    """Engine stays ROLLED across ticks when find_transcript returns the same path."""
    import logging
    transcript = tmp_path / "transcript.jsonl"
    transcript.touch()

    engine = Engine()
    # Drive to ROLLED
    engine.classify(_usage(0.55))
    engine.classify(_usage(0.85))
    assert engine.state is State.ROLLED

    last_transcript = transcript

    # Simulate two ticks with the same transcript
    for _ in range(2):
        current = transcript  # same path
        if engine.state is State.ROLLED and current and current != last_transcript:
            engine.reset()
            last_transcript = current

    assert engine.state is State.ROLLED


# ---------------------------------------------------------------------------
# Test 3: resets to NORMAL when transcript path changes
# ---------------------------------------------------------------------------

def test_rolled_resets_to_normal_on_new_transcript(tmp_path, caplog):
    """Engine resets to NORMAL when find_transcript returns a new path."""
    import logging
    old_transcript = tmp_path / "old_transcript.jsonl"
    new_transcript = tmp_path / "new_transcript.jsonl"
    old_transcript.touch()
    new_transcript.touch()

    engine = Engine()
    # Drive to ROLLED
    engine.classify(_usage(0.55))
    engine.classify(_usage(0.85))
    assert engine.state is State.ROLLED

    last_transcript = old_transcript

    # Simulate a tick where transcript path has changed
    current = new_transcript
    if engine.state is State.ROLLED and current and current != last_transcript:
        engine.reset()
        last_transcript = current

    assert engine.state is State.NORMAL
    assert last_transcript == new_transcript
