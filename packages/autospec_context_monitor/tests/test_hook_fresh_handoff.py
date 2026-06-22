"""Tests for issue #898 Part 2, FIX A: honest fresh-handoff hook notification.

``_run_hook_mode`` (the PreCompact path) is notification-only — it cannot
summarize the session, so it must NOT point the user at a stale handoff from a
prior session. It must:

- surface a FRESH handoff (mtime newer than the session-start cutoff) when one
  exists, via ``notify_clear_needed``;
- surface an HONEST "no fresh handoff — run /create-handoff" message (NOT a
  stale file) when the only handoff predates the session, via
  ``notify_no_fresh_handoff``;
- warn (log + note) when hook mode is the only thing wired (no live daemon).

These tests fail before the fix because the old code always surfaced
``files[-1]`` (newest by mtime) regardless of age and had no
``notify_no_fresh_handoff`` path.
"""
from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path
from unittest.mock import patch

import pytest

from autospec_context_monitor.adapters.base import Usage


def _args(cwd, started_at):
    return argparse.Namespace(
        hook_event="PreCompact",
        tmux_session=None,
        harness=None,
        cwd=str(cwd),
        started_at=started_at,
    )


def _write_handoff(handoff_dir: Path, name: str, mtime: float) -> Path:
    handoff_dir.mkdir(parents=True, exist_ok=True)
    p = handoff_dir / name
    p.write_text(
        "# Handoff\n\n## Status\nwork\n\n## Next step\ngo\n" + ("x" * 200),
        encoding="utf-8",
    )
    os.utime(p, (mtime, mtime))
    return p


@pytest.fixture
def patched_hook(tmp_path, monkeypatch):
    """Redirect monitor dir + stub the adapter's usage/transcript reads."""
    import autospec_context_monitor.__main__ as _main

    monitors = tmp_path / "monitors"
    monkeypatch.setattr(_main, "_MONITORS_DIR", monitors)

    # Stub transcript discovery + usage so we don't need a real Claude session.
    transcript = tmp_path / "transcript.jsonl"
    transcript.write_text("{}", encoding="utf-8")
    os.utime(transcript, (time.time(), time.time()))

    monkeypatch.setattr(
        "autospec_context_monitor.adapters.claude.ClaudeAdapter.find_transcript",
        lambda self, hint: transcript,
    )
    monkeypatch.setattr(
        "autospec_context_monitor.adapters.claude.ClaudeAdapter.read_usage",
        lambda self, t: Usage(
            used_tokens=170_000, max_tokens=200_000, model="m", estimated=False
        ),
    )
    return _main, monitors


def _read_events(logf: Path):
    return [
        json.loads(l)
        for l in logf.read_text().splitlines()
        if l.strip()
    ]


def test_fresh_handoff_is_surfaced(patched_hook, tmp_path):
    """A handoff newer than started_at is surfaced via notify_clear_needed."""
    _main, monitors = patched_hook
    now = time.time()
    handoff_dir = tmp_path / ".turbo" / "handoff"
    today = time.strftime("%Y-%m-%d", time.localtime(now))
    fresh = _write_handoff(handoff_dir, f"{today}-fresh.md", now)  # newer than start

    started_at = now - 60  # session started a minute ago

    from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

    with patch.object(ClaudeHookAdapter, "notify_clear_needed") as m_fresh, \
         patch.object(ClaudeHookAdapter, "notify_no_fresh_handoff") as m_none:
        _main._run_hook_mode(_args(tmp_path, started_at))

    m_fresh.assert_called_once()
    # The fresh handoff path must be the one surfaced.
    _, kwargs = m_fresh.call_args
    surfaced = kwargs.get("handoff_path")
    if surfaced is None and m_fresh.call_args.args:
        surfaced = m_fresh.call_args.args[0]
    assert Path(surfaced) == fresh
    m_none.assert_not_called()

    events = {e["event"] for e in _read_events(monitors / "hook-PreCompact.log")}
    assert "hook_notify" in events


def test_stale_handoff_yields_honest_message(patched_hook, tmp_path):
    """When the only handoff predates the session, surface honest 'no fresh' msg."""
    _main, monitors = patched_hook
    now = time.time()
    handoff_dir = tmp_path / ".turbo" / "handoff"
    today = time.strftime("%Y-%m-%d", time.localtime(now))
    # Stale: written well before the session started.
    stale = _write_handoff(handoff_dir, f"{today}-stale.md", now - 10_000)

    started_at = now - 60  # session started after the stale handoff

    from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

    with patch.object(ClaudeHookAdapter, "notify_clear_needed") as m_fresh, \
         patch.object(ClaudeHookAdapter, "notify_no_fresh_handoff") as m_none:
        _main._run_hook_mode(_args(tmp_path, started_at))

    # MUST NOT point at the stale file.
    m_fresh.assert_not_called()
    m_none.assert_called_once()

    events = _read_events(monitors / "hook-PreCompact.log")
    kinds = {e["event"] for e in events}
    assert "hook_notify_no_fresh" in kinds
    no_fresh = next(e for e in events if e["event"] == "hook_notify_no_fresh")
    assert no_fresh["stale_present"] is True
    assert Path(no_fresh["stale_latest"]) == stale


def test_no_handoff_at_all_yields_honest_message(patched_hook, tmp_path):
    """With no handoff dir at all, surface the honest 'no fresh' message."""
    _main, monitors = patched_hook
    now = time.time()

    from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

    with patch.object(ClaudeHookAdapter, "notify_clear_needed") as m_fresh, \
         patch.object(ClaudeHookAdapter, "notify_no_fresh_handoff") as m_none:
        _main._run_hook_mode(_args(tmp_path, now - 60))

    m_fresh.assert_not_called()
    m_none.assert_called_once()


def test_warns_when_no_daemon_running(patched_hook, tmp_path):
    """Hook-only (no live daemon pidfile) logs a hook_only_no_daemon warning."""
    _main, monitors = patched_hook
    now = time.time()
    handoff_dir = tmp_path / ".turbo" / "handoff"
    today = time.strftime("%Y-%m-%d", time.localtime(now))
    _write_handoff(handoff_dir, f"{today}-fresh.md", now)

    from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

    # No pidfiles in monitors dir => no daemon.
    with patch.object(ClaudeHookAdapter, "notify_clear_needed"), \
         patch.object(ClaudeHookAdapter, "notify_no_fresh_handoff"):
        _main._run_hook_mode(_args(tmp_path, now - 60))

    events = {e["event"] for e in _read_events(monitors / "hook-PreCompact.log")}
    assert "hook_only_no_daemon" in events


def test_no_warning_when_daemon_running(patched_hook, tmp_path):
    """A live daemon pidfile suppresses the hook_only_no_daemon warning."""
    _main, monitors = patched_hook
    now = time.time()
    handoff_dir = tmp_path / ".turbo" / "handoff"
    today = time.strftime("%Y-%m-%d", time.localtime(now))
    _write_handoff(handoff_dir, f"{today}-fresh.md", now)

    # Write a pidfile pointing at THIS process (guaranteed alive).
    monitors.mkdir(parents=True, exist_ok=True)
    (monitors / "live.pid").write_text(str(os.getpid()))

    from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

    with patch.object(ClaudeHookAdapter, "notify_clear_needed"), \
         patch.object(ClaudeHookAdapter, "notify_no_fresh_handoff"):
        _main._run_hook_mode(_args(tmp_path, now - 60))

    events = {e["event"] for e in _read_events(monitors / "hook-PreCompact.log")}
    assert "hook_only_no_daemon" not in events
