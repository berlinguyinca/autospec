"""Tests for autospec_context_monitor.adapters.codex (CodexAdapter)."""
from __future__ import annotations

import time
from pathlib import Path

import pytest

FIXTURES = Path(__file__).parent.parent / "fixtures" / "codex"


@pytest.fixture()
def adapter():
    from autospec_context_monitor.adapters.codex import CodexAdapter
    return CodexAdapter()


def test_read_usage_with_info_populated_uses_explicit_tokens(adapter):
    """When info is populated, uses explicit token counts and estimated=False."""
    fixture = FIXTURES / "info-populated.jsonl"
    usage = adapter.read_usage(fixture)
    # 1200+300 + 800+200 = 2500
    assert usage.used_tokens == 2500
    assert usage.estimated is False
    assert usage.model == "gpt-5"
    assert usage.max_tokens == 200_000


def test_read_usage_with_info_null_falls_back_to_text_length(adapter):
    """When info is null, falls back to text-length estimation and estimated=True."""
    fixture = FIXTURES / "info-null.jsonl"
    usage = adapter.read_usage(fixture)
    assert usage.estimated is True
    assert usage.used_tokens > 0


def test_command_clear_maps_to_slash_new(adapter):
    """command('clear') returns '/new' for Codex CLI."""
    assert adapter.command("clear") == "/new"


def test_command_compact_and_handoff(adapter):
    """compact maps to /compact; handoff contains expected prose."""
    assert adapter.command("compact") == "/compact"
    handoff = adapter.command("handoff")
    assert "handoff" in handoff.lower()
    assert ".turbo/handoff" in handoff


def test_find_transcript_picks_newest_rollout(tmp_path, adapter, monkeypatch):
    """find_transcript returns the most-recently-modified rollout-*.jsonl."""
    from pathlib import Path as _Path

    session_dir = tmp_path / ".codex" / "sessions" / "2026" / "05" / "31"
    session_dir.mkdir(parents=True)

    older = session_dir / "rollout-aaa.jsonl"
    newer = session_dir / "rollout-bbb.jsonl"
    older.write_text('{"type":"event"}\n')
    time.sleep(0.05)
    newer.write_text('{"type":"event"}\n')

    monkeypatch.setattr(_Path, "home", staticmethod(lambda: tmp_path))

    result = adapter.find_transcript({"cwd": "/irrelevant"})
    assert result == newer


def test_find_transcript_raises_when_missing(tmp_path, adapter, monkeypatch):
    """find_transcript raises TranscriptNotFoundError when no rollout files exist."""
    from pathlib import Path as _Path
    from autospec_context_monitor.adapters.base import TranscriptNotFoundError

    monkeypatch.setattr(_Path, "home", staticmethod(lambda: tmp_path))
    with pytest.raises(TranscriptNotFoundError):
        adapter.find_transcript({"cwd": "/any"})
