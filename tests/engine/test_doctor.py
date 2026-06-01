"""Tests for autospec_context_monitor.doctor."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.doctor import main


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _all_passing_patches():
    """Return a context manager stack that makes every check pass."""
    return [
        patch("autospec_context_monitor.doctor._tmux_version", return_value="tmux 3.4"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/claude"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ]


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_doctor_exits_zero_on_healthy_install(capsys):
    """All checks passing → exit 0."""
    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="tmux 3.4"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ):
        rc = main([])
    assert rc == 0
    out = capsys.readouterr().out
    assert "All checks passed" in out


def test_doctor_exits_nonzero_when_tmux_missing(capsys):
    """tmux returning MISSING → exit 1."""
    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="MISSING"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ):
        rc = main([])
    assert rc == 1
    out = capsys.readouterr().out
    assert "FAIL" in out or "failed" in out.lower()


def test_doctor_json_flag_emits_valid_json(capsys):
    """--json flag → stdout is valid JSON array."""
    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="tmux 3.4"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ):
        rc = main(["--json"])
    assert rc == 0
    out = capsys.readouterr().out
    parsed = json.loads(out)
    assert isinstance(parsed, list)
    assert len(parsed) == 8  # one entry per check
    for entry in parsed:
        assert "check" in entry
        assert "result" in entry


def test_self_test_flag_invokes_pytest(capsys):
    """--self-test flag runs pytest and returns its exit code on failure."""
    mock_pytest = MagicMock()
    mock_pytest.main.return_value = 0  # pytest passes

    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="tmux 3.4"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
        patch.dict(sys.modules, {"pytest": mock_pytest}),
    ):
        rc = main(["--self-test"])

    mock_pytest.main.assert_called_once_with(["-q", "tests/adapters", "tests/engine"])
    assert rc == 0


def test_doctor_exits_nonzero_when_adapters_fail(capsys):
    """Adapter import failure → exit 1."""
    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="tmux 3.4"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="IMPORT_ERROR: autospec_context_monitor.adapters.opencode: No module named 'opencode_db'"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ):
        rc = main([])
    assert rc == 1


def test_doctor_json_nonzero_when_check_fails(capsys):
    """--json + failing check → non-zero exit, still emits valid JSON."""
    with (
        patch("autospec_context_monitor.doctor._tmux_version", return_value="MISSING"),
        patch("autospec_context_monitor.doctor._binary", return_value="/usr/bin/tool"),
        patch("autospec_context_monitor.doctor._transcripts_writable", return_value=True),
        patch("autospec_context_monitor.doctor._import_all_adapters", return_value="ok"),
        patch("autospec_context_monitor.doctor._list_pids", return_value=[]),
        patch("autospec_context_monitor.doctor._recent_events", return_value=[]),
    ):
        rc = main(["--json"])
    assert rc == 1
    out = capsys.readouterr().out
    parsed = json.loads(out)
    assert any(e["result"] == "MISSING" for e in parsed)
