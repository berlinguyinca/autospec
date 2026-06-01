"""Tests for autospec_context_monitor.handoff."""
from __future__ import annotations

import time
from datetime import date, timedelta
from pathlib import Path

import pytest


def _write_md(directory: Path, name: str, content: str = "# handoff\n") -> Path:
    """Write a Markdown file to *directory* and return its path."""
    directory.mkdir(parents=True, exist_ok=True)
    p = directory / name
    p.write_text(content)
    return p


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

TODAY = date.today().isoformat()
YESTERDAY = (date.today() - timedelta(days=1)).isoformat()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_returns_newest_file_after_since(tmp_path):
    """wait_for_handoff returns the most-recently-modified file newer than since."""
    from autospec_context_monitor.handoff import wait_for_handoff

    handoff_dir = tmp_path / ".turbo" / "handoff"
    since = time.time() - 1  # anchor in the past so new files qualify

    older = _write_md(handoff_dir, f"{TODAY}-task-a.md")
    time.sleep(0.05)
    newer = _write_md(handoff_dir, f"{TODAY}-task-b.md")

    result = wait_for_handoff(tmp_path, since=since, timeout=5.0, poll=0.05)
    assert result == newer


def test_ignores_files_older_than_since(tmp_path):
    """wait_for_handoff ignores existing files written before the since anchor."""
    from autospec_context_monitor.handoff import wait_for_handoff, HandoffTimeoutError

    handoff_dir = tmp_path / ".turbo" / "handoff"
    # Write a file first, then anchor since AFTER the file's mtime.
    old_file = _write_md(handoff_dir, f"{TODAY}-old-task.md")
    since = old_file.stat().st_mtime + 1  # guarantee it's older than since

    with pytest.raises(HandoffTimeoutError):
        wait_for_handoff(tmp_path, since=since, timeout=0.3, poll=0.05)


def test_raises_handoff_timeout_when_nothing_appears(tmp_path):
    """wait_for_handoff raises HandoffTimeoutError after timeout with no files."""
    from autospec_context_monitor.handoff import wait_for_handoff, HandoffTimeoutError

    since = time.time()
    with pytest.raises(HandoffTimeoutError) as exc_info:
        wait_for_handoff(tmp_path, since=since, timeout=0.3, poll=0.05)

    # Error message should mention the expected directory
    assert ".turbo" in str(exc_info.value)
    assert "handoff" in str(exc_info.value)


def test_handoff_timeout_error_is_timeout_error(tmp_path):
    """HandoffTimeoutError subclasses TimeoutError."""
    from autospec_context_monitor.handoff import HandoffTimeoutError

    assert issubclass(HandoffTimeoutError, TimeoutError)


def test_returns_only_today_dated_files(tmp_path):
    """wait_for_handoff ignores yesterday-prefixed files even if they are newer than since."""
    from autospec_context_monitor.handoff import wait_for_handoff, HandoffTimeoutError

    handoff_dir = tmp_path / ".turbo" / "handoff"
    since = time.time() - 1

    # Write a yesterday-dated file — should be ignored
    _write_md(handoff_dir, f"{YESTERDAY}-stale-task.md")

    with pytest.raises(HandoffTimeoutError):
        wait_for_handoff(tmp_path, since=since, timeout=0.3, poll=0.05)
