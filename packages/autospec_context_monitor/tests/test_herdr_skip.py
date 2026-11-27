"""Herdr-managed sessions are skipped by the context monitor.

Herdr runs its own automatic handoff controller; two policies must never act
on one session. Subprocess tests use real processes, a HOME under tmp_path and
a recording ``tmux`` first on PATH (no mocks).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

from autospec_context_monitor.__main__ import is_herdr_managed

PKG_ROOT = Path(__file__).resolve().parents[1]


def test_is_herdr_managed_true_with_pane_id():
    assert is_herdr_managed({"HERDR_PANE_ID": "w1:p1"}) is True


def test_false_without_pane_id():
    assert is_herdr_managed({}) is False
    assert is_herdr_managed({"HERDR_PANE_ID": ""}) is False


def test_skip_disabled_by_setting_zero():
    env = {"HERDR_PANE_ID": "w1:p1", "AUTOSPEC_CONTEXT_MONITOR_SKIP_HERDR": "0"}
    assert is_herdr_managed(env) is False
    env["AUTOSPEC_CONTEXT_MONITOR_SKIP_HERDR"] = "1"
    assert is_herdr_managed(env) is True


def _run_monitor(
    tmp_path: Path,
    extra_env: dict[str, str],
    args: list[str] | None = None,
) -> tuple[subprocess.CompletedProcess, Path, Path]:
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    bindir = tmp_path / "bin"
    bindir.mkdir()
    tmux_calls = tmp_path / "tmux-calls.txt"
    tmux = bindir / "tmux"
    tmux.write_text(f'#!/bin/sh\necho "$@" >> "{tmux_calls}"\nexit 1\n')
    tmux.chmod(0o755)
    for notifier in ("osascript", "notify-send"):
        stub = bindir / notifier
        stub.write_text(f'#!/bin/sh\necho "$@" >> "{tmux_calls}.{notifier}"\nexit 0\n')
        stub.chmod(0o755)
    env = {
        "HOME": str(home),
        "PATH": f"{bindir}{os.pathsep}{os.environ.get('PATH', '')}",
        "PYTHONPATH": str(PKG_ROOT),
        **extra_env,
    }
    proc = subprocess.run(
        [sys.executable, "-m", "autospec_context_monitor",
         *(args or ["--tmux-session", "s1", "--harness", "claude", "--cwd", str(tmp_path)])],
        env=env, capture_output=True, text=True, timeout=60, cwd=str(tmp_path),
    )
    return proc, home / ".autospec" / "monitors" / "s1.log", tmux_calls


def _events(log: Path) -> list[dict]:
    return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]


def test_monitor_main_exits_zero_and_logs_herdr_managed(tmp_path):
    proc, log, tmux_calls = _run_monitor(tmp_path, {"HERDR_PANE_ID": "w1:p1"})
    assert proc.returncode == 0, proc.stderr
    events = _events(log)
    assert [e["event"] for e in events] == ["herdr_managed"]
    assert "ts" in events[0]
    assert not tmux_calls.exists()


def test_kill_switch_still_works_unchanged(tmp_path):
    home = tmp_path / "home"
    (home / ".autospec").mkdir(parents=True)
    (home / ".autospec" / "no-auto-rollover.flag").touch()
    proc, log, tmux_calls = _run_monitor(tmp_path, {})
    assert proc.returncode == 0, proc.stderr
    names = [e["event"] for e in _events(log)]
    assert names[:2] == ["start", "kill_switch"]
    assert not tmux_calls.exists()


def test_hook_mode_skipped_logs_to_hook_event_log(tmp_path):
    proc, _, tmux_calls = _run_monitor(
        tmp_path, {"HERDR_PANE_ID": "w1:p1"}, args=["--hook-event", "PreCompact"]
    )
    assert proc.returncode == 0, proc.stderr
    log = tmp_path / "home" / ".autospec" / "monitors" / "hook-PreCompact.log"
    assert [e["event"] for e in _events(log)] == ["herdr_managed"]
    assert not (tmp_path / "home" / ".autospec" / "monitors" / "hook.log").exists()
    assert not tmux_calls.exists()
    for notifier in ("osascript", "notify-send"):
        assert not Path(f"{tmux_calls}.{notifier}").exists()


def test_skip_wins_alongside_kill_switch_flag(tmp_path):
    home = tmp_path / "home"
    (home / ".autospec").mkdir(parents=True)
    (home / ".autospec" / "no-auto-rollover.flag").touch()
    proc, log, tmux_calls = _run_monitor(tmp_path, {"HERDR_PANE_ID": "w1:p1"})
    assert proc.returncode == 0, proc.stderr
    assert [e["event"] for e in _events(log)] == ["herdr_managed"]
    assert not tmux_calls.exists()


def test_setting_zero_runs_like_outside_herdr(tmp_path):
    home = tmp_path / "home"
    (home / ".autospec").mkdir(parents=True)
    (home / ".autospec" / "no-auto-rollover.flag").touch()
    proc, log, _ = _run_monitor(
        tmp_path,
        {"HERDR_PANE_ID": "w1:p1", "AUTOSPEC_CONTEXT_MONITOR_SKIP_HERDR": "0"},
    )
    assert proc.returncode == 0, proc.stderr
    assert [e["event"] for e in _events(log)][:2] == ["start", "kill_switch"]
