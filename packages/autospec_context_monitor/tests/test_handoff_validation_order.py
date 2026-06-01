"""Regression tests for g-015: handoff validation must run before cancel window.

Covers:
  - dispatch aborts with a log warning when no handoff file exists (before cancel window)
  - dispatch proceeds to cancel window when a valid handoff file exists
  - cancel-window mock is NOT called when validation fails (ordering regression)
"""
from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import MagicMock, patch, call

import pytest

from autospec_context_monitor.__main__ import _dispatch
from autospec_context_monitor.engine import Action


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_logf(tmp_path: Path) -> Path:
    logf = tmp_path / "test.log"
    logf.touch()
    return logf


def _make_adapter():
    adapter = MagicMock()
    adapter.command.return_value = "/clear"
    adapter.prompt_marker.return_value = None  # no prompt guard
    return adapter


# ---------------------------------------------------------------------------
# Test 1: no handoff file → abort before cancel window
# ---------------------------------------------------------------------------

def test_dispatch_clear_aborts_when_no_handoff_file(tmp_path):
    """When handoff_dir is empty, dispatch aborts early (returns True) without
    invoking the cancel-window."""
    logf = _make_logf(tmp_path)
    handoff_dir = tmp_path / "handoff"
    handoff_dir.mkdir()  # exists but no .md files
    cwd = str(tmp_path)

    with (
        patch("autospec_context_monitor.__main__.wait_for_cancel") as mock_cancel,
        patch("autospec_context_monitor.__main__.session_exists", return_value=True),
        patch("autospec_context_monitor.__main__.inject"),
        patch("autospec_context_monitor.__main__.subprocess.run"),
    ):
        result = _dispatch(
            Action("clear"),
            tmux_session="test-session",
            logf=logf,
            harness="claude",
            cwd=cwd,
            pct=0.85,
            adapter=_make_adapter(),
        )

    assert result is True, "dispatch should abort (return True) when no handoff file found"
    mock_cancel.assert_not_called(), "cancel window must NOT be shown when validation fails"

    # Check that a warning was logged
    log_lines = [json.loads(line) for line in logf.read_text().splitlines() if line.strip()]
    warning_events = [l for l in log_lines if "no_handoff" in l.get("event", "") or "handoff" in l.get("event", "")]
    assert warning_events, f"Expected a handoff warning log entry, got: {log_lines}"


# ---------------------------------------------------------------------------
# Test 2: valid handoff file → proceeds to cancel window
# ---------------------------------------------------------------------------

def test_dispatch_clear_proceeds_to_cancel_window_when_handoff_exists(tmp_path):
    """When a valid handoff file exists, dispatch proceeds to cancel window."""
    logf = _make_logf(tmp_path)
    handoff_dir = tmp_path / ".turbo" / "handoff"
    handoff_dir.mkdir(parents=True)
    handoff_file = handoff_dir / "handoff.md"
    # Write a minimal valid handoff file (validate_handoff checks sections)
    handoff_file.write_text("## Summary\n\n## Status\n\n## Next Steps\n")
    cwd = str(tmp_path)

    with (
        patch("autospec_context_monitor.__main__.wait_for_cancel", return_value=True) as mock_cancel,
        patch("autospec_context_monitor.__main__.validate_handoff", return_value=(True, [])),
        patch("autospec_context_monitor.__main__.session_exists", return_value=True),
        patch("autospec_context_monitor.__main__.inject"),
        patch("autospec_context_monitor.__main__.subprocess.run"),
    ):
        result = _dispatch(
            Action("clear"),
            tmux_session="test-session",
            logf=logf,
            harness="claude",
            cwd=cwd,
            pct=0.85,
            adapter=_make_adapter(),
        )

    mock_cancel.assert_called_once(), "cancel window should be shown when handoff is valid"


# ---------------------------------------------------------------------------
# Test 3: regression — cancel window NOT called when validation fails
# ---------------------------------------------------------------------------

def test_cancel_window_not_called_when_validation_fails(tmp_path):
    """Regression: if handoff validation fails, wait_for_cancel must not be invoked."""
    logf = _make_logf(tmp_path)
    handoff_dir = tmp_path / ".turbo" / "handoff"
    handoff_dir.mkdir(parents=True)
    handoff_file = handoff_dir / "handoff.md"
    handoff_file.write_text("incomplete handoff")
    cwd = str(tmp_path)

    with (
        patch("autospec_context_monitor.__main__.wait_for_cancel") as mock_cancel,
        patch("autospec_context_monitor.__main__.validate_handoff", return_value=(False, ["Summary", "Next Steps"])),
        patch("autospec_context_monitor.__main__.session_exists", return_value=True),
        patch("autospec_context_monitor.__main__.inject"),
        patch("autospec_context_monitor.__main__.subprocess.run"),
    ):
        result = _dispatch(
            Action("clear"),
            tmux_session="test-session",
            logf=logf,
            harness="claude",
            cwd=cwd,
            pct=0.85,
            adapter=_make_adapter(),
        )

    assert result is True
    mock_cancel.assert_not_called(), "cancel window must NOT be called when handoff validation fails"
