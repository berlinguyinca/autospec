"""Unit tests for handoff.validate_handoff.

Uses three fixture files:
- tests/fixtures/handoff/full.md    — valid (>= 200 bytes, all headings)
- tests/fixtures/handoff/stub.md    — too small (< 200 bytes)
- tests/fixtures/handoff/partial.md — missing ## Next step
"""
from __future__ import annotations

from pathlib import Path

import pytest

from autospec_context_monitor.handoff import validate_handoff

_FIXTURES = Path(__file__).parent / "fixtures" / "handoff"


def test_full_handoff_is_valid():
    """full.md must pass all checks and return (True, [])."""
    path = _FIXTURES / "full.md"
    ok, missing = validate_handoff(path)
    assert ok is True
    assert missing == []


def test_stub_handoff_fails_size():
    """stub.md is under 200 bytes; validate_handoff must report 'size' in missing."""
    path = _FIXTURES / "stub.md"
    assert path.stat().st_size < 200, "fixture must be < 200 bytes for this test to be meaningful"
    ok, missing = validate_handoff(path)
    assert ok is False
    assert "size" in missing


def test_partial_handoff_missing_next_step():
    """partial.md has # heading and ## Status but lacks ## Next step."""
    path = _FIXTURES / "partial.md"
    ok, missing = validate_handoff(path)
    assert ok is False
    assert "## Next step" in missing
    # Must NOT report status or heading missing
    assert "## Status" not in missing
    assert "# heading" not in missing
