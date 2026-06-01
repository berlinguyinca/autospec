"""Tests for autospec_context_monitor.adapters.claude (ClaudeAdapter)."""
from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

FIXTURES = Path(__file__).parent.parent / "fixtures" / "claude"


@pytest.fixture()
def adapter():
    from autospec_context_monitor.adapters.claude import ClaudeAdapter
    return ClaudeAdapter()


def test_find_transcript_picks_newest_by_mtime(tmp_path, adapter, monkeypatch):
    """find_transcript returns the most-recently-modified .jsonl in the slug dir."""
    slug = "Users-test-project"
    session_dir = tmp_path / ".claude" / "projects" / slug
    session_dir.mkdir(parents=True)

    older = session_dir / "aaa.jsonl"
    newer = session_dir / "bbb.jsonl"
    older.write_text('{"type":"message"}\n')
    time.sleep(0.05)
    newer.write_text('{"type":"message"}\n')

    monkeypatch.setenv("HOME", str(tmp_path))
    # Patch Path.home() to return tmp_path
    monkeypatch.setattr(Path, "home", staticmethod(lambda: tmp_path))

    result = adapter.find_transcript({"cwd": "/Users/test/project"})
    assert result == newer


def test_find_transcript_raises_when_missing(tmp_path, adapter, monkeypatch):
    """find_transcript raises TranscriptNotFoundError when no .jsonl files exist."""
    from autospec_context_monitor.adapters.base import TranscriptNotFoundError

    monkeypatch.setattr(Path, "home", staticmethod(lambda: tmp_path))
    with pytest.raises(TranscriptNotFoundError):
        adapter.find_transcript({"cwd": "/nonexistent/project"})


def test_read_usage_sums_input_output_tokens(adapter):
    """read_usage correctly sums input+output tokens from short-session.jsonl."""
    fixture = FIXTURES / "short-session.jsonl"
    usage = adapter.read_usage(fixture)
    # 150+50 + 200+75 = 475
    assert usage.used_tokens == 475
    assert usage.model == "claude-sonnet-4-5"
    assert usage.max_tokens == 200_000
    assert usage.estimated is False


def test_read_usage_handles_missing_usage_field(adapter):
    """read_usage returns 0 tokens and doesn't crash when usage field is absent."""
    fixture = FIXTURES / "missing-usage.jsonl"
    usage = adapter.read_usage(fixture)
    assert usage.used_tokens == 0
    assert usage.model == "claude-sonnet-4-5"  # model is still picked up
    assert usage.estimated is False


def test_read_usage_picks_up_1m_model_max(adapter):
    """read_usage returns max_tokens=1_000_000 for the sonnet-1m model."""
    fixture = FIXTURES / "sonnet-1m.jsonl"
    usage = adapter.read_usage(fixture)
    assert usage.max_tokens == 1_000_000
    assert usage.model == "claude-sonnet-4-5-1m"
    assert usage.used_tokens == 6000  # 5000+1000


def test_command_maps_clear_compact_handoff(adapter):
    """command() returns the correct string for each logical command."""
    assert adapter.command("clear") == "/clear"
    assert adapter.command("compact") == "/compact"
    assert "create-handoff" in adapter.command("handoff")
