"""Tests for autospec_context_monitor.adapters.base"""
import dataclasses
import sys
from pathlib import Path
from typing import Literal

import pytest


def test_usage_is_frozen():
    """Assigning to a Usage field must raise FrozenInstanceError."""
    from autospec_context_monitor.adapters.base import Usage

    u = Usage(used_tokens=10, max_tokens=100, model="claude-3-5-sonnet", estimated=False)
    with pytest.raises(dataclasses.FrozenInstanceError):
        u.used_tokens = 99  # type: ignore[misc]


def test_usage_fields():
    """Usage has exactly 4 fields: used_tokens, max_tokens, model, estimated."""
    from autospec_context_monitor.adapters.base import Usage

    fields = {f.name for f in dataclasses.fields(Usage)}
    assert fields == {"used_tokens", "max_tokens", "model", "estimated"}


def test_usage_pct_helper():
    """If pct property/method exists, it returns used/max."""
    from autospec_context_monitor.adapters.base import Usage

    u = Usage(used_tokens=50, max_tokens=100, model="x", estimated=False)
    if hasattr(u, "pct"):
        assert u.pct == 0.5


def test_protocol_runtime_checkable():
    """A concrete class implementing HarnessAdapter passes isinstance check."""
    from autospec_context_monitor.adapters.base import HarnessAdapter, Usage

    class ConcreteAdapter:
        name = "test"

        def find_transcript(self, hint: dict) -> Path:
            return Path("/tmp/transcript.jsonl")

        def read_usage(self, transcript: Path) -> Usage:
            return Usage(used_tokens=10, max_tokens=100, model="x", estimated=False)

        def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
            return f"/{logical}"

        def prompt_marker(self) -> str:
            return "> "

    adapter = ConcreteAdapter()
    assert isinstance(adapter, HarnessAdapter)


def test_transcript_not_found_is_filenotfound_subclass():
    """TranscriptNotFoundError must be a subclass of FileNotFoundError."""
    from autospec_context_monitor.adapters.base import TranscriptNotFoundError

    assert issubclass(TranscriptNotFoundError, FileNotFoundError)


def test_import_from_package_root():
    """All three public names are importable from autospec_context_monitor directly."""
    from autospec_context_monitor import HarnessAdapter, TranscriptNotFoundError, Usage

    assert Usage is not None
    assert HarnessAdapter is not None
    assert TranscriptNotFoundError is not None
