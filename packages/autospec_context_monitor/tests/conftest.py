"""Shared pytest configuration for autospec_context_monitor tests.

Provides an autouse fixture that stubs the ``subprocess`` name inside
``autospec_context_monitor.__main__`` so that tmux display-message overlay
calls (added in g-009) don't require a real tmux binary during the unit-test
suite.

IMPORTANT: we replace the *module-level name* ``autospec_context_monitor.__main__.subprocess``
with a ``MagicMock`` rather than patching ``subprocess.run`` directly on the
real ``subprocess`` module.  Patching the real module would affect every
``subprocess.run`` call in the process, including calls in test helpers such as
``install.sh`` invocations via ``subprocess.run(["bash", ...])``.
"""
from __future__ import annotations

import types
from unittest.mock import MagicMock

import pytest


@pytest.fixture(autouse=True)
def _stub_subprocess_run(monkeypatch):
    """Replace the subprocess name in __main__ with a stub so tmux calls are no-ops."""
    import autospec_context_monitor.__main__ as _main

    fake_subprocess = MagicMock()
    fake_subprocess.run = MagicMock(return_value=MagicMock(returncode=0))
    monkeypatch.setattr(_main, "subprocess", fake_subprocess)
