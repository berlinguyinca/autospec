"""Tests for autospec_context_monitor.adapters.opencode (OpenCodeAdapter)."""
from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from unittest.mock import MagicMock

import pytest

FIXTURES = Path(__file__).parent.parent / "fixtures" / "opencode"


@pytest.fixture()
def adapter():
    from autospec_context_monitor.adapters.opencode import OpenCodeAdapter
    return OpenCodeAdapter()


def test_read_usage_sums_tokens_for_session_id(adapter):
    """read_usage sums input+output tokens only for the matching session_id."""
    fixture = FIXTURES / "session.db"
    adapter._session_id = "sess-abc"
    usage = adapter.read_usage(fixture)
    # sess-abc: (1500+300) + (800+200) = 2800; sess-other excluded
    assert usage.used_tokens == 2800
    assert usage.model == "claude-sonnet-4-6"
    assert usage.estimated is False
    assert usage.max_tokens == 200_000


def test_db_opened_with_mode_ro(adapter, monkeypatch):
    """read_usage opens the database URI containing literal 'mode=ro'."""
    fixture = FIXTURES / "session.db"
    adapter._session_id = "sess-abc"

    captured_uris: list[str] = []
    original_connect = sqlite3.connect

    def spy_connect(*args, **kwargs):
        # First positional arg is the URI string; keyword 'uri=True' marks it as a URI
        db_arg = args[0] if args else kwargs.get("database", "")
        if kwargs.get("uri"):
            captured_uris.append(db_arg)
        return original_connect(*args, **kwargs)

    monkeypatch.setattr(sqlite3, "connect", spy_connect)
    adapter.read_usage(fixture)

    assert captured_uris, "sqlite3.connect was not called with a URI"
    assert any("mode=ro" in u for u in captured_uris), (
        f"Expected 'mode=ro' in URI but got: {captured_uris}"
    )


def test_read_usage_falls_back_when_schema_changed(adapter, monkeypatch):
    """read_usage calls opencode export fallback when messages table is missing."""
    fixture = FIXTURES / "schema-changed.db"
    adapter._session_id = "sess-abc"

    # Mock subprocess.run to return JSON lines with token data
    export_output = json.dumps({"input_tokens": 400, "output_tokens": 80, "model": "claude-sonnet-4-6"})

    mock_result = MagicMock()
    mock_result.stdout = export_output + "\n"

    import subprocess
    monkeypatch.setattr(subprocess, "run", lambda *a, **kw: mock_result)

    usage = adapter.read_usage(fixture)
    assert usage.used_tokens == 480
    assert usage.model == "claude-sonnet-4-6"
    assert usage.estimated is False


def test_command_clear_maps_to_slash_new(adapter):
    """command('clear') returns '/new' for OpenCode."""
    assert adapter.command("clear") == "/new"


def test_command_compact_maps_to_slash_compact(adapter):
    """command('compact') returns '/compact'."""
    assert adapter.command("compact") == "/compact"


def test_command_handoff_contains_turbo_path(adapter):
    """command('handoff') mentions the .turbo/handoff path."""
    result = adapter.command("handoff")
    assert ".turbo/handoff" in result


def test_find_transcript_raises_when_db_missing(tmp_path, adapter, monkeypatch):
    """find_transcript raises TranscriptNotFoundError when db doesn't exist."""
    from pathlib import Path as _Path
    from autospec_context_monitor.adapters.base import TranscriptNotFoundError
    from autospec_context_monitor.adapters import opencode as oc_module

    monkeypatch.setattr(oc_module, "_DB_PATH", tmp_path / "nonexistent.db")
    with pytest.raises(TranscriptNotFoundError):
        adapter.find_transcript({"session_id": "abc"})


def test_find_transcript_stores_session_id(tmp_path, adapter, monkeypatch):
    """find_transcript caches session_id from hint for use in read_usage."""
    from pathlib import Path as _Path
    from autospec_context_monitor.adapters import opencode as oc_module

    fake_db = tmp_path / "opencode.db"
    fake_db.touch()
    monkeypatch.setattr(oc_module, "_DB_PATH", fake_db)

    adapter.find_transcript({"session_id": "my-session"})
    assert adapter._session_id == "my-session"
