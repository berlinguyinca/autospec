"""Unit tests for the ClaudeHookAdapter (PreCompact hook mode).

Four tests:
1. command("clear") returns HOOK_NOTIFY_SENTINEL.
2. read_usage output matches ClaudeAdapter for the same transcript.
3. install.sh --hook-mode claude merges keys without clobbering existing ones.
4. Running install.sh --hook-mode claude twice is idempotent (same content).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

from autospec_context_monitor.adapters.claude_hook import (
    HOOK_NOTIFY_SENTINEL,
    ClaudeHookAdapter,
)
from autospec_context_monitor.adapters.claude import ClaudeAdapter

_INSTALL_SH = Path(__file__).resolve().parents[4] / "install.sh"
_PKG_ROOT = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------------
# Test 1 — command("clear") returns sentinel
# ---------------------------------------------------------------------------

def test_command_clear_returns_sentinel():
    """ClaudeHookAdapter.command('clear') must return HOOK_NOTIFY_SENTINEL."""
    adapter = ClaudeHookAdapter()
    result = adapter.command("clear")
    assert result == HOOK_NOTIFY_SENTINEL
    assert result == "__HOOK_NOTIFY__"


# ---------------------------------------------------------------------------
# Test 2 — read_usage matches ClaudeAdapter
# ---------------------------------------------------------------------------

def test_read_usage_matches_base_adapter(tmp_path):
    """ClaudeHookAdapter.read_usage must produce the same result as ClaudeAdapter."""
    # Minimal JSONL transcript with usage data.
    transcript = tmp_path / "session.jsonl"
    transcript.write_text(
        json.dumps({
            "message": {
                "model": "claude-sonnet-4-6",
                "usage": {"input_tokens": 1000, "output_tokens": 500},
            }
        }) + "\n",
        encoding="utf-8",
    )

    hook_adapter = ClaudeHookAdapter()
    base_adapter = ClaudeAdapter()

    hook_usage = hook_adapter.read_usage(transcript)
    base_usage = base_adapter.read_usage(transcript)

    assert hook_usage.used_tokens == base_usage.used_tokens
    assert hook_usage.max_tokens == base_usage.max_tokens
    assert hook_usage.model == base_usage.model
    assert hook_usage.estimated == base_usage.estimated


# ---------------------------------------------------------------------------
# Test 3 — install.sh --hook-mode claude merges without clobbering
# ---------------------------------------------------------------------------

def test_install_hook_mode_claude_merges_keys(tmp_path, monkeypatch):
    """install.sh --hook-mode claude must add hooks without removing existing keys."""
    settings = tmp_path / ".claude" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text(
        json.dumps({"theme": "dark", "existingKey": "preserved"}),
        encoding="utf-8",
    )

    env = {**os.environ, "HOME": str(tmp_path)}
    result = subprocess.run(
        ["bash", str(_INSTALL_SH), "--hook-mode", "claude"],
        env=env,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"install.sh failed:\n{result.stderr}"

    data = json.loads(settings.read_text())
    assert data.get("theme") == "dark", "existing 'theme' key must be preserved"
    assert data.get("existingKey") == "preserved", "existing keys must not be clobbered"
    assert "PreCompact" in data.get("hooks", {}), "hooks.PreCompact must be added"
    assert "SessionStart" in data.get("hooks", {}), "hooks.SessionStart must be added"
    # Verify the monitor command is present in the hook lists. Claude Code's
    # canonical schema nests each entry as
    # {"hooks": [{"type": "command", "command": "..."}]}; descend into the
    # nested step commands rather than substring-matching a flat string.
    pre_cmds = [
        step.get("command", "")
        for entry in data["hooks"]["PreCompact"]
        for step in entry.get("hooks", [])
    ]
    assert any("autospec_context_monitor" in cmd and "PreCompact" in cmd for cmd in pre_cmds)


# ---------------------------------------------------------------------------
# Test 4 — install.sh --hook-mode claude is idempotent
# ---------------------------------------------------------------------------

def test_install_hook_mode_claude_idempotent(tmp_path):
    """Running install.sh --hook-mode claude twice must produce identical output."""
    settings = tmp_path / ".claude" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text("{}", encoding="utf-8")

    # Isolate the subprocess so the surrounding harness's own hooks can't mutate
    # the test's tmp HOME/.claude/settings.json *between* the two install runs
    # (install.sh's writes are themselves idempotent — the flakiness is purely
    # external mutation). Point Claude Code's config dir away from tmp HOME and
    # disable harness hooks inside the subprocess.
    env = {
        **os.environ,
        "HOME": str(tmp_path),
        "CLAUDE_CONFIG_DIR": str(tmp_path / "claude-config-isolated"),
        "OMC_SKIP_HOOKS": "1",
        "DISABLE_OMC": "1",
    }
    cmd = ["bash", str(_INSTALL_SH), "--hook-mode", "claude"]

    r1 = subprocess.run(cmd, env=env, capture_output=True, text=True)
    assert r1.returncode == 0, f"first run failed:\n{r1.stderr}"
    content_after_first = settings.read_text()

    r2 = subprocess.run(cmd, env=env, capture_output=True, text=True)
    assert r2.returncode == 0, f"second run failed:\n{r2.stderr}"
    content_after_second = settings.read_text()

    assert content_after_first == content_after_second, (
        "settings.json must be identical after two runs (idempotent)"
    )
