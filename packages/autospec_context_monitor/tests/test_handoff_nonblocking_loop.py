"""Tests for issue #898 Part 2, FIX B: poll loop not frozen by handoff wait.

Previously ``_dispatch(Action('handoff'))`` called ``wait_for_handoff(timeout=180)``
synchronously, blocking the ENTIRE poll loop for up to 180s. Now the handoff
wait is a deadline-based pending state advanced one non-blocking tick at a time,
so the loop keeps checking kill-switch / session-alive / usage while a handoff
is pending.

These tests fail before the fix:
- the responsiveness test would hang (or block on real ``wait_for_handoff``)
  instead of processing a kill-switch tick during the handoff window;
- the timeout test relied on ``_advance_pending_handoff`` which did not exist.
"""
from __future__ import annotations

import time
from datetime import date
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

import autospec_context_monitor.__main__ as _main
from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Action


def _usage(used, max_tokens=200_000):
    return Usage(used_tokens=used, max_tokens=max_tokens, model="m", estimated=False)


def _args(tmp_path, session="sess"):
    return [
        "--tmux-session", session,
        "--harness", "claude",
        "--cwd", str(tmp_path),
        "--started-at", "0",
    ]


@pytest.fixture(autouse=True)
def _redirect(tmp_path, monkeypatch):
    monitors = tmp_path / "monitors"
    monkeypatch.setattr(_main, "_MONITORS_DIR", monitors)
    monkeypatch.setattr(_main, "_KILL_SWITCH", tmp_path / "no-auto-rollover.flag")
    monkeypatch.setattr(_main, "_POLL_INTERVAL", 0)  # no real sleeping
    return monitors


def _write_valid_handoff(tmp_path) -> Path:
    hd = tmp_path / ".turbo" / "handoff"
    hd.mkdir(parents=True, exist_ok=True)
    p = hd / f"{date.today().isoformat()}-h.md"
    p.write_text(
        "# Handoff\n\n## Status\nok\n\n## Next step\ngo\n" + ("x" * 250),
        encoding="utf-8",
    )
    return p


# ---------------------------------------------------------------------------
# FIX B core: the loop stays responsive while a handoff is pending.
# ---------------------------------------------------------------------------

def test_loop_responsive_while_handoff_pending(tmp_path):
    """While the handoff is pending, the loop keeps polling and honors the
    kill-switch on a later tick — instead of blocking 180s in one call."""
    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    # Climb straight to >=80%: NORMAL->COMPACTED(compact) tick 0, then
    # COMPACTED->ROLLED([handoff,clear,resume]) tick 1.
    fake_adapter.read_usage.return_value = _usage(170_000)
    fake_adapter.command.side_effect = lambda k: f"/{k}"
    fake_adapter.prompt_marker.return_value = ""

    tick = {"n": 0}

    def session_alive(_sess):
        # Stay alive for many ticks; the kill-switch (set below) ends the loop,
        # proving the loop is NOT blocked inside a single 180s handoff wait.
        tick["n"] += 1
        if tick["n"] == 4:
            # Drop the kill switch on the 4th liveness check — well within the
            # handoff window — to demonstrate responsiveness.
            (tmp_path / "no-auto-rollover.flag").touch()
        return True

    inject_calls = []

    with patch.object(_main, "session_exists", side_effect=session_alive), \
         patch.object(_main, "inject", side_effect=lambda s, c: inject_calls.append(c)), \
         patch.object(_main, "wait_for_prompt", return_value=True), \
         patch.object(_main, "_load_adapter", return_value=fake_adapter), \
         patch.object(_main.time, "sleep", lambda *_: None):
        _main.main(_args(tmp_path))

    # The handoff command was injected, but /clear was NOT (no handoff file ever
    # appeared, and the kill-switch ended the loop mid-wait). Crucially the loop
    # ran several ticks rather than blocking once for 180s.
    assert any("handoff" in c for c in inject_calls), inject_calls
    assert not any("clear" in c for c in inject_calls), inject_calls
    assert tick["n"] >= 4, f"loop did not keep polling while pending (ticks={tick['n']})"


# ---------------------------------------------------------------------------
# Happy path: handoff appears -> clear + resume dispatched.
# ---------------------------------------------------------------------------

def test_handoff_appears_then_clear_and_resume(tmp_path):
    """Once a fresh handoff file appears, the deferred clear+resume run."""
    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    fake_adapter.read_usage.return_value = _usage(170_000)
    fake_adapter.command.side_effect = lambda k: f"/{k}"
    fake_adapter.prompt_marker.return_value = ""

    tick = {"n": 0}

    def session_alive(_sess):
        tick["n"] += 1
        # On tick 3 (handoff already injected & pending), the harness "writes"
        # the handoff file; next pending-advance tick should observe it.
        if tick["n"] == 3:
            _write_valid_handoff(tmp_path)
        if tick["n"] >= 8:
            (tmp_path / "no-auto-rollover.flag").touch()
        return True

    inject_calls = []

    with patch.object(_main, "session_exists", side_effect=session_alive), \
         patch.object(_main, "inject", side_effect=lambda s, c: inject_calls.append(c)), \
         patch.object(_main, "wait_for_prompt", return_value=True), \
         patch.object(_main, "wait_for_cancel", return_value=False), \
         patch.object(_main, "_load_adapter", return_value=fake_adapter), \
         patch.object(_main.time, "sleep", lambda *_: None):
        _main.main(_args(tmp_path))

    assert any("handoff" in c for c in inject_calls), inject_calls
    assert any("clear" in c for c in inject_calls), inject_calls
    # resume is injected as prose, not via command(); check it ran by message.
    assert any("continue from where" in c.lower() for c in inject_calls), inject_calls


# ---------------------------------------------------------------------------
# Timeout path: no handoff before deadline -> revert to COMPACTED.
# ---------------------------------------------------------------------------

def test_handoff_timeout_reverts_to_compacted(tmp_path):
    """_advance_pending_handoff returns ('timeout', True) past the deadline and
    records a handoff_timeout; the loop reverts engine state to COMPACTED."""
    from autospec_context_monitor.engine import State

    logf = tmp_path / "log.jsonl"
    pending = _main._PendingHandoff(
        since=time.time(),
        deadline=100.0,  # monotonic deadline
        rest=[Action("clear"), Action("resume")],
    )

    recorded = []
    with patch.object(_main._stats, "record", side_effect=lambda *a, **k: recorded.append((a, k))):
        # Before the deadline (now < deadline) and no file: still pending.
        status, canceled = _main._advance_pending_handoff(
            pending, tmp_path, logf,
            harness="claude", tmux_session="s", cwd=str(tmp_path),
            pct=0.85, adapter=MagicMock(), now=50.0,
        )
        assert status == "pending" and canceled is False

        # Past the deadline and still no file: timeout + canceled.
        status, canceled = _main._advance_pending_handoff(
            pending, tmp_path, logf,
            harness="claude", tmux_session="s", cwd=str(tmp_path),
            pct=0.85, adapter=MagicMock(), now=101.0,
        )
        assert status == "timeout" and canceled is True

    assert any(a[0] == "handoff_timeout" for a, _ in recorded), recorded

    # And the full loop reverts to COMPACTED on timeout. Drive a tiny loop that
    # times out fast via a small deadline through the real main().
    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    fake_adapter.read_usage.return_value = _usage(170_000)
    fake_adapter.command.side_effect = lambda k: f"/{k}"
    fake_adapter.prompt_marker.return_value = ""

    captured_state = {}
    tick = {"n": 0}

    def session_alive(_sess):
        tick["n"] += 1
        if tick["n"] >= 6:
            (tmp_path / "no-auto-rollover.flag").touch()
        return True

    with patch.object(_main, "_HANDOFF_DEADLINE", 0.0), \
         patch.object(_main, "session_exists", side_effect=session_alive), \
         patch.object(_main, "inject", lambda s, c: None), \
         patch.object(_main, "wait_for_prompt", return_value=True), \
         patch.object(_main, "_load_adapter", return_value=fake_adapter), \
         patch.object(_main.time, "sleep", lambda *_: None):
        _main.main(_args(tmp_path, "timeout-sess"))

    # Read the log: a handoff_timeout + a state_reverted->COMPACTED must appear.
    import json
    logf2 = tmp_path / "monitors" / "timeout-sess.log"
    events = [json.loads(l) for l in logf2.read_text().splitlines() if l.strip()]
    kinds = [e["event"] for e in events]
    assert "handoff_timeout" in kinds, kinds
    reverts = [e for e in events if e["event"] == "state_reverted"]
    assert reverts and reverts[-1]["state"] == "COMPACTED", events
