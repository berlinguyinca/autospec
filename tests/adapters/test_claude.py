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
    # Slug keeps the leading "-" from the absolute cwd's leading "/" (the live
    # ~/.claude/projects convention; claude.py derives it via re.sub and the
    # docstring says do NOT strip it). The fixture dir MUST match or the test
    # rots (it did — this test was failing on main, un-gated, before the fix).
    slug = "-Users-test-project"
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


@pytest.mark.parametrize(
    "model,expected",
    [
        ("claude-opus-4-8", 200_000),          # base opus-4-8 (non-1m)
        ("claude-opus-4-8-1m", 1_000_000),     # explicit 1m variant
        ("claude-opus-4-8[1m]", 1_000_000),    # [1m]-tagged exact id (issue #898)
        ("claude-sonnet-4-5-1m", 1_000_000),   # existing 1m model still resolves
        ("claude-sonnet-4-5", 200_000),        # existing base model
        ("some-future-model-1m", 1_000_000),   # unknown id, -1m suffix → fallback to 1M
        ("some-future-model[1m]", 1_000_000),  # unknown id, [1m] tag → fallback to 1M
        ("totally-unknown-model", 200_000),    # unknown, no marker → default
        (None, 200_000),                       # no model field → default
    ],
)
def test_resolve_max_covers_opus48_and_1m_family(model, expected):
    """_resolve_max maps opus-4-8 + any -1m/[1m]-tagged id to 1M, else 200k default."""
    from autospec_context_monitor.adapters.claude import _resolve_max
    assert _resolve_max(model) == expected


def test_read_usage_opus48_1m_not_over_capped(tmp_path, adapter):
    """Regression for #898: a 344k-token opus-4-8[1m] session reads as 1M context
    (pct < 50%), not the 200k default that reported >100% and fired early rollover."""
    transcript = tmp_path / "opus48-1m.jsonl"
    transcript.write_text(
        '{"type":"message","message":{"model":"claude-opus-4-8[1m]",'
        '"usage":{"input_tokens":300000,"output_tokens":44000}}}\n'
    )
    usage = adapter.read_usage(transcript)
    assert usage.model == "claude-opus-4-8[1m]"
    assert usage.max_tokens == 1_000_000
    assert usage.used_tokens == 344_000
    assert (usage.used_tokens / usage.max_tokens) < 0.5  # ~34%, not 172%


def test_command_maps_clear_compact_handoff(adapter):
    """command() returns the correct string for each logical command."""
    assert adapter.command("clear") == "/clear"
    assert adapter.command("compact") == "/compact"
    assert "create-handoff" in adapter.command("handoff")
