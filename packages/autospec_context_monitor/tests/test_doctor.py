"""Unit tests for autospec_context_monitor.doctor.

Covers four AC items from issue #750:
- test_doctor_exits_zero_when_healthy
- test_doctor_exits_nonzero_when_tmux_missing
- test_doctor_json_output_is_valid_json
- test_doctor_self_test_invokes_pytest
"""
from __future__ import annotations

import json
import subprocess
import sys
from unittest.mock import MagicMock, patch

import pytest

import autospec_context_monitor.doctor as doctor


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_ok_check():
    """Return a callable that always reports a non-failing value."""
    return lambda: "ok"


def _patch_all_checks_healthy(monkeypatch):
    """Monkeypatch _build_checks so every check returns a healthy value."""
    healthy_checks = [
        ("tmux", lambda: "tmux 3.3"),
        ("claude binary", lambda: "/usr/local/bin/claude"),
        ("codex binary", lambda: "/usr/local/bin/codex"),
        ("opencode binary", lambda: "/usr/local/bin/opencode"),
        ("transcripts writable", lambda: True),
        ("adapters importable", lambda: "ok"),
        ("monitor PIDs", lambda: []),
        ("recent events", lambda: []),
    ]
    monkeypatch.setattr(doctor, "_build_checks", lambda: healthy_checks)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_doctor_exits_zero_when_healthy(monkeypatch):
    """main([]) returns 0 when all checks pass."""
    _patch_all_checks_healthy(monkeypatch)
    result = doctor.main([])
    assert result == 0


def test_doctor_exits_nonzero_when_tmux_missing(monkeypatch):
    """main([]) returns non-zero when tmux check reports MISSING."""
    failing_checks = [
        ("tmux", lambda: "MISSING"),
        ("claude binary", lambda: "/usr/local/bin/claude"),
        ("codex binary", lambda: "/usr/local/bin/codex"),
        ("opencode binary", lambda: "/usr/local/bin/opencode"),
        ("transcripts writable", lambda: True),
        ("adapters importable", lambda: "ok"),
        ("monitor PIDs", lambda: []),
        ("recent events", lambda: []),
    ]
    monkeypatch.setattr(doctor, "_build_checks", lambda: failing_checks)
    result = doctor.main([])
    assert result != 0


def test_doctor_json_output_is_valid_json(monkeypatch, capsys):
    """main(['--json']) emits valid JSON to stdout."""
    _patch_all_checks_healthy(monkeypatch)
    doctor.main(["--json"])
    captured = capsys.readouterr()
    # Must parse without raising
    parsed = json.loads(captured.out)
    assert isinstance(parsed, list)
    assert len(parsed) > 0
    # Each entry has 'check' and 'result' keys
    for entry in parsed:
        assert "check" in entry
        assert "result" in entry


def test_doctor_self_test_invokes_pytest(monkeypatch):
    """main(['--self-test']) calls pytest.main when all checks pass."""
    _patch_all_checks_healthy(monkeypatch)

    pytest_calls = []

    def fake_pytest_main(args):
        pytest_calls.append(args)
        return 0

    fake_pytest = MagicMock()
    fake_pytest.main = fake_pytest_main

    # Patch pytest import inside doctor
    monkeypatch.setattr("builtins.__import__", _make_importer(fake_pytest))

    # Use a simpler approach: patch sys.modules so doctor picks up our mock
    import sys
    original_pytest = sys.modules.get("pytest")
    sys.modules["pytest"] = fake_pytest
    try:
        result = doctor.main(["--self-test"])
    finally:
        if original_pytest is None:
            sys.modules.pop("pytest", None)
        else:
            sys.modules["pytest"] = original_pytest

    assert len(pytest_calls) == 1
    assert result == 0


def _make_importer(fake_pytest):
    """Return a custom __import__ that returns fake_pytest for 'pytest'."""
    real_import = __builtins__.__import__ if hasattr(__builtins__, "__import__") else __import__

    def custom_import(name, *args, **kwargs):
        if name == "pytest":
            return fake_pytest
        return real_import(name, *args, **kwargs)

    return custom_import
