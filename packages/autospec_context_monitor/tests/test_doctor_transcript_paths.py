"""Tests for g-012: doctor _transcripts_writable iterates per-adapter PATHS_TO_CHECK.

Verifies that:
- _transcripts_writable reports entries for codex and opencode.
- Missing PATHS_TO_CHECK on an adapter does not raise.
"""
from __future__ import annotations

import types
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest


# ---------------------------------------------------------------------------
# Test 1: _transcripts_writable reports codex and opencode entries
# ---------------------------------------------------------------------------

def test_transcripts_writable_includes_codex_and_opencode(tmp_path):
    """_transcripts_writable must include entries for codex and opencode adapters."""
    from autospec_context_monitor.doctor import _transcripts_writable

    # The function reads PATHS_TO_CHECK from real adapter modules.
    result = _transcripts_writable()

    assert isinstance(result, list), "_transcripts_writable must return a list"
    labels = [entry.split(":")[0] for entry in result]
    assert "codex" in labels, f"codex must appear in results; got: {result}"
    assert "opencode" in labels, f"opencode must appear in results; got: {result}"
    assert "claude" in labels, f"claude must appear in results; got: {result}"

    # Each entry must have the form "label:path:status"
    for entry in result:
        parts = entry.split(":")
        assert len(parts) >= 3, f"Entry must be 'label:path:status'; got: {entry!r}"
        status = parts[-1]
        assert status in ("writable", "NOT_WRITABLE"), (
            f"Status must be 'writable' or 'NOT_WRITABLE'; got: {status!r}"
        )


# ---------------------------------------------------------------------------
# Test 2: adapters without PATHS_TO_CHECK do not raise
# ---------------------------------------------------------------------------

def test_transcripts_writable_skips_adapter_without_paths_to_check(monkeypatch):
    """_transcripts_writable gracefully skips adapters with no PATHS_TO_CHECK."""
    import autospec_context_monitor.doctor as doctor_mod

    # Build a fake adapter module with no PATHS_TO_CHECK attribute.
    fake_mod = types.ModuleType("autospec_context_monitor.adapters.fake")
    # (no PATHS_TO_CHECK attribute)

    original_modules = doctor_mod._ADAPTER_MODULES
    monkeypatch.setattr(doctor_mod, "_ADAPTER_MODULES", [
        "autospec_context_monitor.adapters.fake",
    ])

    import sys
    sys.modules["autospec_context_monitor.adapters.fake"] = fake_mod
    try:
        result = doctor_mod._transcripts_writable()
    finally:
        del sys.modules["autospec_context_monitor.adapters.fake"]

    # Must return an empty list without raising.
    assert result == [], f"Expected empty list for adapter with no PATHS_TO_CHECK; got: {result}"


# ---------------------------------------------------------------------------
# Test 3: PATHS_TO_CHECK is defined on codex and opencode
# ---------------------------------------------------------------------------

def test_adapter_modules_export_paths_to_check():
    """codex and opencode adapter modules must export PATHS_TO_CHECK."""
    from autospec_context_monitor.adapters import codex, opencode, claude

    for mod, name in [(codex, "codex"), (opencode, "opencode"), (claude, "claude")]:
        assert hasattr(mod, "PATHS_TO_CHECK"), (
            f"{name} adapter must define PATHS_TO_CHECK"
        )
        paths = mod.PATHS_TO_CHECK
        assert isinstance(paths, list), f"{name}.PATHS_TO_CHECK must be a list"
        assert len(paths) > 0, f"{name}.PATHS_TO_CHECK must be non-empty"
        assert all(isinstance(p, Path) for p in paths), (
            f"{name}.PATHS_TO_CHECK entries must be Path objects"
        )
