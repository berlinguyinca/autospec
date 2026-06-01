"""Shared pytest configuration for autospec_context_monitor tests.

Provides a module-scoped autouse fixture that stubs subprocess.run so that
tmux display-message overlay calls (added in g-009) don't require a real
tmux binary during the unit-test suite.
"""
from __future__ import annotations

from unittest.mock import MagicMock

import pytest


@pytest.fixture(autouse=True)
def _stub_subprocess_run(monkeypatch):
    """Stub subprocess.run so tmux overlay calls succeed without a real tmux binary."""
    import autospec_context_monitor.__main__ as _main

    monkeypatch.setattr(_main.subprocess, "run", MagicMock(return_value=MagicMock(returncode=0)))
