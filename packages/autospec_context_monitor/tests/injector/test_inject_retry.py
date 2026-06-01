"""Tests for _inject_with_retry — retry logic and bell/notify fallback.

All subprocess.run calls are mocked so no real tmux/osascript/notify-send
processes are spawned.
"""
from __future__ import annotations

import shutil
from unittest.mock import MagicMock, call, patch

import pytest

import autospec_context_monitor.injector as injector_mod
from autospec_context_monitor.injector import _inject_with_retry


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_result(returncode: int) -> MagicMock:
    r = MagicMock()
    r.returncode = returncode
    return r


# ---------------------------------------------------------------------------
# Test 1: succeeds on first attempt — no sleep, returns True
# ---------------------------------------------------------------------------

def test_inject_with_retry_success_first_attempt():
    """First subprocess.run succeeds → no sleep, returns True."""
    cmd = ["tmux", "send-keys", "-l", "-t", "mysession", "hello"]
    with (
        patch("autospec_context_monitor.injector.subprocess.run", return_value=_make_result(0)) as mock_run,
        patch("autospec_context_monitor.injector.time.sleep") as mock_sleep,
    ):
        result = _inject_with_retry(cmd, "test_action")

    assert result is True
    assert mock_run.call_count == 1
    mock_sleep.assert_not_called()


# ---------------------------------------------------------------------------
# Test 2: fails first, succeeds on retry — sleep called once, returns True
# ---------------------------------------------------------------------------

def test_inject_with_retry_success_on_second_attempt():
    """First call fails, second call succeeds → sleep(5) called once, returns True."""
    cmd = ["tmux", "send-keys", "-l", "-t", "mysession", "hello"]
    side_effects = [_make_result(1), _make_result(0)]
    with (
        patch("autospec_context_monitor.injector.subprocess.run", side_effect=side_effects) as mock_run,
        patch("autospec_context_monitor.injector.time.sleep") as mock_sleep,
    ):
        result = _inject_with_retry(cmd, "test_action")

    assert result is True
    assert mock_run.call_count == 2
    mock_sleep.assert_called_once_with(5)


# ---------------------------------------------------------------------------
# Test 3: fails both attempts — tput bel called, notification sent, returns False
# ---------------------------------------------------------------------------

def test_inject_with_retry_both_fail_rings_bell():
    """Both attempts fail → tput bel called, returns False."""
    cmd = ["tmux", "send-keys", "-l", "-t", "mysession", "hello"]

    run_calls: list[list] = []

    def fake_run(args, **kwargs):
        run_calls.append(list(args))
        return _make_result(1)

    with (
        patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run),
        patch("autospec_context_monitor.injector.time.sleep"),
        patch("autospec_context_monitor.injector.shutil.which", return_value=None),
    ):
        result = _inject_with_retry(cmd, "my_action")

    assert result is False
    # tput bel must appear in one of the calls
    tput_calls = [c for c in run_calls if c and c[0] == "tput"]
    assert tput_calls, "Expected tput bel call but none found"
    assert tput_calls[0] == ["tput", "bel"]


def test_inject_with_retry_both_fail_notification_message_contains_action():
    """Both attempts fail → notification message contains the action name."""
    cmd = ["tmux", "send-keys", "-l", "-t", "mysession", "hello"]
    action = "inject_context"

    run_calls: list[list] = []

    def fake_run(args, **kwargs):
        run_calls.append(list(args))
        return _make_result(1)

    # Simulate macOS: osascript available, notify-send absent
    def fake_which(prog):
        return "/usr/bin/osascript" if prog == "osascript" else None

    with (
        patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run),
        patch("autospec_context_monitor.injector.time.sleep"),
        patch("autospec_context_monitor.injector.shutil.which", side_effect=fake_which),
    ):
        result = _inject_with_retry(cmd, action)

    assert result is False
    # Find osascript call
    osascript_calls = [c for c in run_calls if c and c[0] == "osascript"]
    assert osascript_calls, "Expected osascript call"
    notification_arg = " ".join(osascript_calls[0])
    assert action in notification_arg, f"Action name not in notification: {notification_arg}"
    assert "[autospec]" in notification_arg


def test_inject_with_retry_both_fail_notify_send_fallback():
    """Both attempts fail on Linux → notify-send called as fallback."""
    cmd = ["tmux", "send-keys", "-l", "-t", "mysession", "hello"]
    action = "inject_context"

    run_calls: list[list] = []

    def fake_run(args, **kwargs):
        run_calls.append(list(args))
        return _make_result(1)

    # Simulate Linux: osascript absent, notify-send present
    def fake_which(prog):
        return "/usr/bin/notify-send" if prog == "notify-send" else None

    with (
        patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run),
        patch("autospec_context_monitor.injector.time.sleep"),
        patch("autospec_context_monitor.injector.shutil.which", side_effect=fake_which),
    ):
        result = _inject_with_retry(cmd, action)

    assert result is False
    notify_calls = [c for c in run_calls if c and c[0] == "notify-send"]
    assert notify_calls, "Expected notify-send call"
    assert action in " ".join(notify_calls[0])
