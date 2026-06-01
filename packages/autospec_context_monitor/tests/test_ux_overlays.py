"""Unit tests for tmux display-message overlays in _dispatch (g-009).

Verifies that:
- Each of compact/handoff/clear/resume fires subprocess.run with the correct
  display-message string.
- No subprocess.run call is made when tmux_session is falsy.
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock, call, patch

import pytest

from autospec_context_monitor.__main__ import _dispatch
from autospec_context_monitor.engine import Action


def _make_logf(tmp_path: Path) -> Path:
    return tmp_path / "test.log"


def _fake_adapter(kind: str) -> MagicMock:
    adapter = MagicMock()
    adapter.command.side_effect = lambda k: f"/{k}"
    return adapter


# ---------------------------------------------------------------------------
# Helpers that stub out side-effectful deps so _dispatch can complete.
# ---------------------------------------------------------------------------

_PATCH_INJECT = "autospec_context_monitor.__main__.inject"
_PATCH_WAIT_HANDOFF = "autospec_context_monitor.__main__.wait_for_handoff"
_PATCH_WAIT_CANCEL = "autospec_context_monitor.__main__.wait_for_cancel"
_PATCH_SUBPROCESS = "autospec_context_monitor.__main__.subprocess"


class _FakeHandoffPath:
    """Minimal stand-in so wait_for_handoff result can be str()-ed."""
    def __str__(self):
        return "/tmp/fake-handoff.md"


# ---------------------------------------------------------------------------
# Overlay message tests (one per action kind)
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("kind,expected_msg", [
    ("compact", "[autospec] compacting context…"),
    ("handoff", "[autospec] writing handoff…"),
    ("clear",   "[autospec] clearing session…"),
    ("resume",  "[autospec] resuming…"),
])
def test_overlay_message_for_action(tmp_path, kind, expected_msg):
    """_dispatch calls tmux display-message with the correct overlay text."""
    logf = _make_logf(tmp_path)

    # For 'clear' we need a valid handoff dir with a file so validation passes.
    if kind == "clear":
        handoff_dir = tmp_path / ".turbo" / "handoff"
        handoff_dir.mkdir(parents=True)
        # Create a dummy handoff file big enough to pass validation.
        (handoff_dir / "2026-05-31-handoff.md").write_text("x" * 300)

    with patch(_PATCH_INJECT):
        with patch(_PATCH_WAIT_HANDOFF, return_value=_FakeHandoffPath()):
            with patch(_PATCH_WAIT_CANCEL, return_value=False):
                with patch(_PATCH_SUBPROCESS) as sp_mock:
                    sp_mock.run = MagicMock(return_value=MagicMock(returncode=0))
                    _dispatch(
                        Action(kind),
                        "test-session",
                        logf,
                        cwd=str(tmp_path),
                        adapter=_fake_adapter(kind),
                    )

    # Find the display-message call among all subprocess.run calls.
    display_calls = [
        c for c in sp_mock.run.call_args_list
        if c.args and len(c.args[0]) >= 2 and c.args[0][1] == "display-message"
    ]
    assert display_calls, (
        f"subprocess.run must be called with 'tmux display-message' for action '{kind}'"
    )
    cmd_args = display_calls[0].args[0]
    assert cmd_args == ["tmux", "display-message", "-d", "3000", expected_msg], (
        f"display-message args mismatch for '{kind}': got {cmd_args}"
    )
    assert display_calls[0].kwargs.get("check") is False, (
        "display-message must use check=False"
    )


# ---------------------------------------------------------------------------
# No overlay when tmux_session is falsy
# ---------------------------------------------------------------------------

def test_no_overlay_when_no_tmux_session(tmp_path):
    """No tmux display-message call when tmux_session is empty/None."""
    logf = _make_logf(tmp_path)

    with patch(_PATCH_INJECT):
        with patch(_PATCH_SUBPROCESS) as sp_mock:
            sp_mock.run = MagicMock(return_value=MagicMock(returncode=0))
            # Use compact — simplest branch, no extra deps.
            _dispatch(
                Action("compact"),
                "",  # falsy tmux_session
                logf,
                adapter=_fake_adapter("compact"),
            )

    display_calls = [
        c for c in sp_mock.run.call_args_list
        if c.args and len(c.args[0]) >= 2 and c.args[0][1] == "display-message"
    ]
    assert not display_calls, (
        "No tmux display-message should be issued when tmux_session is empty"
    )
