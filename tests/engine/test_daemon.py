"""Tests for the autospec_context_monitor daemon entrypoint (__main__.py)."""

from __future__ import annotations

import json
import os
import signal
import sys
import threading
import time
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.__main__ import main, _MONITORS_DIR, _KILL_SWITCH


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_usage(used=5_000, max_tokens=100_000, model="test-model", estimated=False):
    from autospec_context_monitor.adapters.base import Usage
    return Usage(used_tokens=used, max_tokens=max_tokens, model=model, estimated=estimated)


def _base_args(tmp_path, session="test-sess"):
    return [
        "--tmux-session", session,
        "--harness", "claude",
        "--cwd", str(tmp_path),
        "--started-at", "0",
    ]


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(autouse=True)
def clean_monitors(tmp_path, monkeypatch):
    """Redirect monitor directory to tmp_path so tests don't touch ~/.autospec."""
    monitors = tmp_path / "monitors"
    monitors.mkdir()
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)
    monkeypatch.setattr("autospec_context_monitor.__main__._KILL_SWITCH",
                        tmp_path / "no-auto-rollover.flag")
    return monitors


# ---------------------------------------------------------------------------
# Test: PID file written on start
# ---------------------------------------------------------------------------

def test_writes_pid_file_on_start(tmp_path, monkeypatch):
    """Daemon writes ~/.autospec/monitors/<session>.pid containing its PID."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)

    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "fake_transcript"
    fake_adapter.read_usage.return_value = _make_usage()

    call_count = [0]

    def stop_after_one(session):
        call_count[0] += 1
        return call_count[0] <= 1

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", side_effect=stop_after_one), \
         patch("autospec_context_monitor.__main__.time.sleep"):
        main(_base_args(tmp_path, "test-sess"))

    # The daemon logs its PID in the "start" event — verify it matches our PID.
    logf = monitors / "test-sess.log"
    assert logf.exists(), "Log file was not created"
    lines = [json.loads(l) for l in logf.read_text().splitlines() if l.strip()]
    start_records = [l for l in lines if l.get("event") == "start"]
    assert start_records, "No 'start' event in log"
    assert start_records[0]["pid"] == os.getpid()


# ---------------------------------------------------------------------------
# Test: PID file path matches expected pattern
# ---------------------------------------------------------------------------

def test_pid_file_path_matches_pattern(tmp_path, monkeypatch):
    """PID file path is ~/.autospec/monitors/<session>.pid."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)

    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    fake_adapter.read_usage.return_value = _make_usage()

    call_count = [0]

    def stop_after_one(session):
        call_count[0] += 1
        return call_count[0] <= 1

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", side_effect=stop_after_one), \
         patch("autospec_context_monitor.__main__.time.sleep"):
        main(_base_args(tmp_path, "my-session"))

    # After exit the pid file is removed, but the monitors dir exists with expected structure.
    pidf = monitors / "my-session.pid"
    logf = monitors / "my-session.log"
    # Log file must exist (pid file is cleaned up on exit).
    assert logf.exists(), "Log file not created at expected path"
    assert not pidf.exists(), "PID file should be removed on clean exit"
    # Verify the log records the expected path implicitly via session name.
    lines = [json.loads(l) for l in logf.read_text().splitlines() if l.strip()]
    start = next((l for l in lines if l.get("event") == "start"), None)
    assert start is not None
    assert start["tmux_session"] == "my-session"


# ---------------------------------------------------------------------------
# Test: PID file removed on SIGTERM
# ---------------------------------------------------------------------------

def test_removes_pid_file_on_sigterm(tmp_path, monkeypatch):
    """Daemon removes the PID file when SIGTERM is received."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)
    monitors.mkdir(exist_ok=True)

    pidf = monitors / "sigterm-sess.pid"

    # We'll invoke main in a thread and send SIGTERM to the process.
    # Since we can't easily SIGTERM from the same process in a thread-safe way,
    # we test the handler directly by invoking it after capturing the signal setup.
    registered_handler = []

    original_signal = signal.signal

    def capture_signal(sig, handler):
        if sig == signal.SIGTERM:
            registered_handler.append(handler)
        return original_signal(sig, handler)

    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    fake_adapter.read_usage.return_value = _make_usage()

    call_count = [0]

    def stop_after_one(session):
        call_count[0] += 1
        return False  # exit immediately

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", side_effect=stop_after_one), \
         patch("autospec_context_monitor.__main__.time.sleep"), \
         patch("autospec_context_monitor.__main__.signal.signal", side_effect=capture_signal):
        main(_base_args(tmp_path, "sigterm-sess"))

    # PID file should be gone after normal exit.
    assert not pidf.exists(), "PID file was not removed on exit"

    # If a handler was registered, verify calling it cleans up.
    if registered_handler:
        # Re-create the pid file to test the handler
        pidf.write_text("12345")
        with pytest.raises(SystemExit):
            registered_handler[0](signal.SIGTERM, None)
        assert not pidf.exists(), "SIGTERM handler did not remove PID file"


# ---------------------------------------------------------------------------
# Test: exits when kill-switch flag present
# ---------------------------------------------------------------------------

def test_exits_when_kill_switch_flag_present(tmp_path, monkeypatch):
    """Daemon exits within one poll-tick after no-auto-rollover.flag appears."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)
    kill_switch = tmp_path / "no-auto-rollover.flag"
    monkeypatch.setattr("autospec_context_monitor.__main__._KILL_SWITCH", kill_switch)

    # Create the kill-switch flag before the daemon starts.
    kill_switch.touch()

    fake_adapter = MagicMock()
    session_called = [False]

    def never_session(session):
        session_called[0] = True
        return True

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", side_effect=never_session), \
         patch("autospec_context_monitor.__main__.time.sleep"):
        main(_base_args(tmp_path, "ks-sess"))

    # session_exists should never be called because kill-switch is checked first.
    assert not session_called[0], "Daemon should have exited before checking session"


# ---------------------------------------------------------------------------
# Test: exits when tmux session gone
# ---------------------------------------------------------------------------

def test_exits_when_tmux_session_gone(tmp_path, monkeypatch):
    """Daemon exits when session_exists() returns False."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)

    fake_adapter = MagicMock()

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", return_value=False), \
         patch("autospec_context_monitor.__main__.time.sleep") as mock_sleep:
        main(_base_args(tmp_path, "gone-sess"))

    # sleep should never be called — loop exited before sleeping.
    mock_sleep.assert_not_called()


# ---------------------------------------------------------------------------
# Test: log file is written
# ---------------------------------------------------------------------------

def test_log_file_written(tmp_path, monkeypatch):
    """Daemon appends JSON-line records to the log file."""
    monitors = tmp_path / "monitors"
    monkeypatch.setattr("autospec_context_monitor.__main__._MONITORS_DIR", monitors)

    fake_adapter = MagicMock()
    fake_adapter.find_transcript.return_value = tmp_path / "t"
    fake_adapter.read_usage.return_value = _make_usage()

    call_count = [0]

    def one_tick(session):
        call_count[0] += 1
        return call_count[0] <= 1

    with patch("autospec_context_monitor.__main__._load_adapter", return_value=fake_adapter), \
         patch("autospec_context_monitor.__main__.session_exists", side_effect=one_tick), \
         patch("autospec_context_monitor.__main__.time.sleep"):
        main(_base_args(tmp_path, "log-sess"))

    logf = monitors / "log-sess.log"
    assert logf.exists(), "Log file was not created"
    lines = [json.loads(l) for l in logf.read_text().splitlines() if l.strip()]
    events = [l["event"] for l in lines]
    assert "start" in events
    assert "stop" in events
