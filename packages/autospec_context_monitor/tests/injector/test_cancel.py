"""Unit tests for injector.wait_for_cancel.

Three scenarios:
1. No cancel — countdown expires, returns False.
2. Cancel mid-countdown — sentinel appears on tick 2, returns True.
3. Exception during countdown — unbind always runs (finally block).
"""
from __future__ import annotations

import subprocess
from pathlib import Path
from unittest.mock import MagicMock, call, patch

import pytest

from autospec_context_monitor.injector import wait_for_cancel


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_run_mock(sentinel_on_tick: int | None = None, sentinel_path_holder: list | None = None):
    """Return a mock for subprocess.run that optionally creates the sentinel.

    ``sentinel_on_tick`` is 1-based: the tick (sleep call) after which the
    sentinel is created.
    """
    tick_counter = {"n": 0}

    def _side_effect(cmd, **_kwargs):
        # Detect sleep-equivalent: display-message call = one tick
        if isinstance(cmd, list) and "display-message" in cmd:
            tick_counter["n"] += 1
            if sentinel_on_tick is not None and tick_counter["n"] >= sentinel_on_tick:
                if sentinel_path_holder:
                    sentinel_path_holder[0].touch()
        result = MagicMock()
        result.returncode = 0
        return result

    return MagicMock(side_effect=_side_effect)


# ---------------------------------------------------------------------------
# Test 1: No cancel — full countdown, returns False
# ---------------------------------------------------------------------------

def test_no_cancel_returns_false(tmp_path, monkeypatch):
    """When the sentinel never appears, wait_for_cancel returns False."""
    monitors_dir = tmp_path / "monitors"
    monkeypatch.setattr(
        "autospec_context_monitor.injector.Path.home",
        lambda: tmp_path,
    )

    run_mock = MagicMock(return_value=MagicMock(returncode=0))
    with patch("autospec_context_monitor.injector.subprocess.run", run_mock):
        with patch("autospec_context_monitor.injector.time.sleep"):
            result = wait_for_cancel("my-sess", timeout=3)

    assert result is False

    # Verify unbind-key was called
    unbind_calls = [
        c for c in run_mock.call_args_list
        if "unbind-key" in c.args[0]
    ]
    assert len(unbind_calls) == 1, "unbind-key must be called exactly once"


# ---------------------------------------------------------------------------
# Test 2: Cancel mid-countdown — sentinel appears, returns True
# ---------------------------------------------------------------------------

def test_cancel_mid_countdown_returns_true(tmp_path, monkeypatch):
    """When the sentinel file appears during polling, wait_for_cancel returns True."""
    monkeypatch.setattr(
        "autospec_context_monitor.injector.Path.home",
        lambda: tmp_path,
    )

    sentinel_holder: list[Path] = []

    def _capture_sentinel_path(cmd, **_kwargs):
        # Capture sentinel path from bind-key run-shell argument
        if isinstance(cmd, list) and "bind-key" in cmd:
            for arg in cmd:
                if "touch" in str(arg):
                    path = Path(arg.split()[-1])
                    sentinel_holder.append(path)
        return MagicMock(returncode=0)

    tick = {"n": 0}

    def _sleep(_s):
        tick["n"] += 1
        # Create sentinel on second tick
        if tick["n"] >= 2 and sentinel_holder:
            sentinel_holder[0].parent.mkdir(parents=True, exist_ok=True)
            sentinel_holder[0].touch()

    run_mock = MagicMock(side_effect=_capture_sentinel_path)
    with patch("autospec_context_monitor.injector.subprocess.run", run_mock):
        with patch("autospec_context_monitor.injector.time.sleep", _sleep):
            result = wait_for_cancel("my-sess", timeout=10)

    assert result is True

    # Verify unbind-key was called even after early exit
    unbind_calls = [
        c for c in run_mock.call_args_list
        if isinstance(c.args[0], list) and "unbind-key" in c.args[0]
    ]
    assert len(unbind_calls) == 1, "unbind-key must always run"


# ---------------------------------------------------------------------------
# Test 3: Exception during countdown — unbind still runs
# ---------------------------------------------------------------------------

def test_unbind_runs_on_exception(tmp_path, monkeypatch):
    """If display-message raises, unbind-key is still called via finally."""
    monkeypatch.setattr(
        "autospec_context_monitor.injector.Path.home",
        lambda: tmp_path,
    )

    call_log: list[list] = []

    def _raising_run(cmd, **_kwargs):
        call_log.append(list(cmd))
        if "display-message" in cmd:
            raise subprocess.TimeoutExpired(cmd, 1)
        return MagicMock(returncode=0)

    with patch("autospec_context_monitor.injector.subprocess.run", _raising_run):
        with patch("autospec_context_monitor.injector.time.sleep"):
            with pytest.raises(subprocess.TimeoutExpired):
                wait_for_cancel("my-sess", timeout=5)

    # unbind-key must appear in call_log despite exception
    unbind_issued = any("unbind-key" in cmd for cmd in call_log)
    assert unbind_issued, "unbind-key must be issued in finally even on exception"
