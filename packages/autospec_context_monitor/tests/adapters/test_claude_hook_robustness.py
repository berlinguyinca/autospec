"""Robustness tests for the PreCompact hook notification path.

The PreCompact hook runs synchronously inside Claude Code, which blocks on it.
``notify_clear_needed`` shells out to ``osascript`` / ``notify-send``; without a
``timeout`` a blocked notifier (no GUI session, pending TCC prompt, wedged
NotificationCenter) freezes the harness through compaction. It must also escape
the handoff path before embedding it in an AppleScript double-quoted string, and
the installed PreCompact hook must be non-blocking (``async``) and time-bounded.
"""
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from autospec_context_monitor.adapters.claude_hook import ClaudeHookAdapter

_INSTALL_SH = Path(__file__).resolve().parents[4] / "install.sh"


def test_notify_passes_timeout(monkeypatch):
    """notify_clear_needed must bound the notifier subprocess with timeout=."""
    calls = {}

    def fake_run(*args, **kwargs):
        calls.update(kwargs)
        return subprocess.CompletedProcess(args, 0)

    monkeypatch.setattr(subprocess, "run", fake_run)
    ClaudeHookAdapter.notify_clear_needed(handoff_path=Path("/tmp/h.md"))
    assert "timeout" in calls, "subprocess.run must be called with a timeout="
    assert calls["timeout"] and calls["timeout"] <= 10


def test_notify_swallows_timeout(monkeypatch):
    """A blocked/timed-out notifier must never propagate out of the hook path."""
    def boom(*args, **kwargs):
        raise subprocess.TimeoutExpired(cmd="osascript", timeout=5)

    monkeypatch.setattr(subprocess, "run", boom)
    # Must not raise.
    ClaudeHookAdapter.notify_clear_needed(handoff_path=Path("/tmp/h.md"))


def test_notify_escapes_applescript_body(monkeypatch):
    """A handoff path with a quote must be escaped, not injected raw into the script."""
    monkeypatch.setattr("sys.platform", "darwin")
    captured = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv
        return subprocess.CompletedProcess(argv, 0)

    monkeypatch.setattr(subprocess, "run", fake_run)
    ClaudeHookAdapter.notify_clear_needed(handoff_path=Path('/tmp/we"ird.md'))

    script = captured["argv"][-1]
    # The raw unescaped sequence  "ird.md"  must not appear; it must be escaped.
    assert '\\"' in script, "double-quote in body must be backslash-escaped"
    assert 'we"ird' not in script, "raw unescaped quote must not reach osascript"


def test_install_precompact_hook_is_async_and_bounded(tmp_path):
    """install.sh --hook-mode claude must register PreCompact as async + timeout."""
    settings = tmp_path / ".claude" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text("{}", encoding="utf-8")

    env = {**os.environ, "HOME": str(tmp_path)}
    r = subprocess.run(
        ["bash", str(_INSTALL_SH), "--hook-mode", "claude"],
        env=env, capture_output=True, text=True,
    )
    assert r.returncode == 0, f"install.sh failed:\n{r.stderr}"

    data = json.loads(settings.read_text())
    pre_entries = data["hooks"]["PreCompact"]
    steps = [s for e in pre_entries for s in e.get("hooks", [])
             if "autospec_context_monitor" in s.get("command", "")]
    assert steps, "PreCompact must contain the monitor step"
    step = steps[0]
    assert step.get("async") is True, "PreCompact monitor step must be async"
    assert isinstance(step.get("timeout"), int) and step["timeout"] > 0, \
        "PreCompact monitor step must have a positive timeout"
