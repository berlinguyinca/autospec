"""Tests for g-011: SessionStart hook handler in _run_hook_mode.

Verifies that:
- _run_hook_mode with hook_event="SessionStart" returns without error (Option A: no-op).
- install.sh --hook-mode claude registers SessionStart in settings.json.
- The registration and handler are consistent (both present).
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

import pytest


# ---------------------------------------------------------------------------
# Test 1: _run_hook_mode("SessionStart") returns without error
# ---------------------------------------------------------------------------

def test_run_hook_mode_session_start_noop(tmp_path):
    """_run_hook_mode with SessionStart fires without raising; logs hook_noop event."""
    from autospec_context_monitor.__main__ import _run_hook_mode

    args = argparse.Namespace(
        hook_event="SessionStart",
        tmux_session=None,
        harness=None,
        cwd=None,
        started_at=None,
    )

    # Redirect _MONITORS_DIR to tmp_path so logs don't pollute ~/.autospec/monitors.
    import autospec_context_monitor.__main__ as _main
    original_dir = _main._MONITORS_DIR
    _main._MONITORS_DIR = tmp_path / "monitors"
    try:
        # Must return without raising.
        _run_hook_mode(args)
    finally:
        _main._MONITORS_DIR = original_dir

    # Log file must contain hook_noop event.
    logf = tmp_path / "monitors" / "hook-SessionStart.log"
    assert logf.exists(), "hook-SessionStart.log must be written"
    events = [json.loads(l)["event"] for l in logf.read_text().splitlines() if l.strip()]
    assert "hook_noop" in events, (
        "SessionStart handler must log a hook_noop event; events: " + str(events)
    )


# ---------------------------------------------------------------------------
# Test 2: install.sh --hook-mode claude registers SessionStart
# ---------------------------------------------------------------------------

_INSTALL_SH = Path(__file__).resolve().parents[3] / "install.sh"


@pytest.mark.skipif(not _INSTALL_SH.exists(), reason="install.sh not found")
def test_install_sh_registers_session_start():
    """install.sh --hook-mode claude must add hooks.SessionStart to settings.json."""
    with tempfile.TemporaryDirectory() as tmp_home:
        settings = Path(tmp_home) / ".claude" / "settings.json"
        settings.parent.mkdir(parents=True)
        settings.write_text("{}", encoding="utf-8")

        env = {**os.environ, "HOME": tmp_home}
        result = subprocess.run(
            ["bash", str(_INSTALL_SH), "--hook-mode", "claude"],
            env=env,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"install.sh failed:\n{result.stderr}"

        data = json.loads(settings.read_text())
        hooks = data.get("hooks", {})
        assert "SessionStart" in hooks, (
            "hooks.SessionStart must be registered by --hook-mode claude"
        )
        # Claude Code's canonical schema nests each entry as
        # {"hooks": [{"type": "command", "command": "..."}]}; descend into the
        # nested step commands rather than substring-matching a flat string.
        sess_entries = hooks["SessionStart"]
        sess_cmds = [
            step.get("command", "")
            for entry in sess_entries
            for step in entry.get("hooks", [])
        ]
        assert any(
            "autospec_context_monitor" in cmd and "SessionStart" in cmd
            for cmd in sess_cmds
        ), f"SessionStart hook must invoke autospec_context_monitor; got: {sess_entries}"


@pytest.mark.skipif(not _INSTALL_SH.exists(), reason="install.sh not found")
def test_install_hook_mode_does_not_require_cargo(tmp_path):
    """Hook-only installation exits before any Rust runtime build."""
    marker = tmp_path / "cargo-invoked"
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake_cargo = bin_dir / "cargo"
    fake_cargo.write_text(
        "#!/usr/bin/env sh\nprintf invoked > \"$CARGO_MARKER\"\nexit 91\n",
        encoding="utf-8",
    )
    fake_cargo.chmod(0o755)

    env = {
        **os.environ,
        "HOME": str(tmp_path),
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "CARGO_MARKER": str(marker),
    }
    result = subprocess.run(
        ["bash", str(_INSTALL_SH), "--hook-mode", "claude"],
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert not marker.exists(), "hook-only installation must not invoke cargo"
