"""Unit tests for prompt-ready guards (g-010).

Covers:
- wait_for_prompt returns True when marker is present on first poll.
- wait_for_prompt returns False after all attempts exhausted.
- _dispatch skips inject when wait_for_prompt returns False.
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.engine import Action
from autospec_context_monitor.injector import wait_for_prompt


# ---------------------------------------------------------------------------
# wait_for_prompt unit tests
# ---------------------------------------------------------------------------

_PATCH_PROMPT_READY = "autospec_context_monitor.injector.prompt_ready"


def test_wait_for_prompt_returns_true_when_marker_found():
    """wait_for_prompt returns True immediately when prompt_ready succeeds."""
    with patch(_PATCH_PROMPT_READY, return_value=True) as mock_pr:
        result = wait_for_prompt("sess", "> ", attempts=3, delay=0.0)

    assert result is True
    mock_pr.assert_called_once_with("sess", "> ", retries=3, wait=0.0)


def test_wait_for_prompt_returns_false_when_exhausted():
    """wait_for_prompt returns False when prompt_ready always fails."""
    with patch(_PATCH_PROMPT_READY, return_value=False) as mock_pr:
        result = wait_for_prompt("sess", "> ", attempts=2, delay=0.0)

    assert result is False
    mock_pr.assert_called_once_with("sess", "> ", retries=2, wait=0.0)


# ---------------------------------------------------------------------------
# _dispatch prompt guard integration tests
# ---------------------------------------------------------------------------

_PATCH_INJECT = "autospec_context_monitor.__main__.inject"
_PATCH_WAIT_FOR_PROMPT = "autospec_context_monitor.__main__.wait_for_prompt"


class _MarkerAdapter:
    """Fake adapter that reports a prompt marker."""

    name = "fake"

    def command(self, logical: str) -> str:
        return f"/{logical}"

    def prompt_marker(self) -> str:
        return "> "


def test_dispatch_skips_inject_when_prompt_not_ready(tmp_path):
    """_dispatch returns False and skips inject when wait_for_prompt returns False."""
    from autospec_context_monitor.__main__ import _dispatch

    logf = tmp_path / "test.log"
    inject_calls: list = []

    def _fake_inject(session, text, **_kw):
        inject_calls.append(text)

    with patch(_PATCH_INJECT, _fake_inject):
        with patch(_PATCH_WAIT_FOR_PROMPT, return_value=False):
            result = _dispatch(
                Action("compact"),
                "test-sess",
                logf,
                adapter=_MarkerAdapter(),
            )

    assert result is False, "_dispatch must return False (not canceled) when prompt guard fails"
    assert not inject_calls, "inject must NOT be called when wait_for_prompt returns False"

    # Log must contain the prompt_not_ready event
    import json
    log_lines = [json.loads(l) for l in logf.read_text().splitlines() if l.strip()]
    events = [l["event"] for l in log_lines]
    assert "prompt_not_ready" in events, "Log must record prompt_not_ready event"


def test_dispatch_proceeds_when_prompt_ready(tmp_path):
    """_dispatch calls inject when wait_for_prompt returns True."""
    from autospec_context_monitor.__main__ import _dispatch

    logf = tmp_path / "test.log"
    inject_calls: list = []

    def _fake_inject(session, text, **_kw):
        inject_calls.append(text)

    with patch(_PATCH_INJECT, _fake_inject):
        with patch(_PATCH_WAIT_FOR_PROMPT, return_value=True):
            _dispatch(
                Action("compact"),
                "test-sess",
                logf,
                adapter=_MarkerAdapter(),
            )

    assert inject_calls, "inject must be called when wait_for_prompt returns True"
