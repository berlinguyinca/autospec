"""Regression tests for g-019: PID file cleanup on all error paths.

Ensures the PID file is removed even when _load_adapter raises an exception,
preventing stale lock files that would cause false "daemon already running"
errors on restart.
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import patch

import pytest


def _run_main_with_args(argv: list[str]) -> None:
    from autospec_context_monitor.__main__ import main
    main(argv)


def _base_argv(tmp_path: Path, session: str = "test-session") -> list[str]:
    return [
        "--tmux-session", session,
        "--harness", "claude",
        "--cwd", str(tmp_path),
        "--started-at", "1748736000.0",
    ]


# ---------------------------------------------------------------------------
# g-019: PID file absent after _load_adapter raises ImportError
# ---------------------------------------------------------------------------

def test_pid_removed_on_load_adapter_import_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """PID file must not exist after _load_adapter raises ImportError."""
    monitors_dir = tmp_path / "monitors"
    monitors_dir.mkdir()
    monkeypatch.setattr(
        "autospec_context_monitor.__main__._MONITORS_DIR",
        monitors_dir,
    )

    def _raise_import(*_args, **_kwargs):
        raise ImportError("simulated missing adapter module")

    with patch("autospec_context_monitor.__main__._load_adapter", side_effect=_raise_import):
        with pytest.raises(ImportError, match="simulated missing adapter module"):
            _run_main_with_args(_base_argv(tmp_path, session="sess-import-err"))

    pid_file = monitors_dir / "sess-import-err.pid"
    assert not pid_file.exists(), f"PID file {pid_file} should have been removed after ImportError"


# ---------------------------------------------------------------------------
# g-019: PID file absent after _load_adapter raises ValueError
# ---------------------------------------------------------------------------

def test_pid_removed_on_load_adapter_value_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """PID file must not exist after _load_adapter raises ValueError."""
    monitors_dir = tmp_path / "monitors"
    monitors_dir.mkdir()
    monkeypatch.setattr(
        "autospec_context_monitor.__main__._MONITORS_DIR",
        monitors_dir,
    )

    def _raise_value(*_args, **_kwargs):
        raise ValueError("Unknown harness: 'bad-harness'")

    with patch("autospec_context_monitor.__main__._load_adapter", side_effect=_raise_value):
        with pytest.raises(ValueError, match="Unknown harness"):
            _run_main_with_args(_base_argv(tmp_path, session="sess-value-err"))

    pid_file = monitors_dir / "sess-value-err.pid"
    assert not pid_file.exists(), f"PID file {pid_file} should have been removed after ValueError"
