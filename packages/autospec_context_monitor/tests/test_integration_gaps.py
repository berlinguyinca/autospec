"""Regression tests for integration gaps g-001…g-006.

Each test corresponds to a gap from the 2026-06-01 audit
(~/.autospec/gaps-20260601T0707Z-b1a71cd.json) and locks in the fix.

Gaps covered:
- g-001: --hook-event {PreCompact,SessionStart} argparse flag; --tmux-session
  optional when --hook-event is set.
- g-003: handoff_dir resolves under <cwd>/.turbo/handoff (not ~/.turbo/handoff).
- g-004: wait_for_handoff is called between Action('handoff') and Action('clear').
- g-005: engine never transitions NORMAL → ROLLED directly; compaction always
  runs first.
- g-006: _dispatch routes every slash injection through adapter.command(...).
"""
from __future__ import annotations

import time
from pathlib import Path
from typing import Literal
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.adapters.base import Usage
from autospec_context_monitor.engine import Action, Engine, State


# ---------------------------------------------------------------------------
# g-001 — --hook-event argparse flag (makes --tmux-session optional)
# ---------------------------------------------------------------------------


def test_g001_hook_event_argparse_accepts_precompact():
    """argparse accepts `--hook-event PreCompact` with no --tmux-session."""
    from autospec_context_monitor.__main__ import _build_parser

    parser = _build_parser()
    args = parser.parse_args(["--hook-event", "PreCompact"])
    assert args.hook_event == "PreCompact"
    # tmux-session/harness/cwd default to None when in hook mode
    assert args.tmux_session in (None, "")


def test_g001_hook_event_argparse_accepts_sessionstart():
    """argparse accepts `--hook-event SessionStart`."""
    from autospec_context_monitor.__main__ import _build_parser

    parser = _build_parser()
    args = parser.parse_args(["--hook-event", "SessionStart"])
    assert args.hook_event == "SessionStart"


def test_g001_hook_event_argparse_rejects_unknown_event():
    """argparse rejects unknown hook events."""
    from autospec_context_monitor.__main__ import _build_parser

    parser = _build_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(["--hook-event", "Nope"])


def test_g001_tmux_session_still_required_without_hook_event():
    """Without --hook-event, --tmux-session/--harness/--cwd are still required.

    Validation happens after parsing (so hook-mode invocations can skip the
    tmux args entirely); we exercise the full main() entry path which calls
    both ``_build_parser`` and ``_validate_args``.
    """
    from autospec_context_monitor.__main__ import _build_parser, _validate_args

    parser = _build_parser()
    args = parser.parse_args([])  # parse succeeds because all flags default to None
    with pytest.raises(SystemExit):
        _validate_args(parser, args)


def test_g001_hook_event_skips_tmux_requirement():
    """`--hook-event PreCompact` alone passes _validate_args (tmux flags optional)."""
    from autospec_context_monitor.__main__ import _build_parser, _validate_args

    parser = _build_parser()
    args = parser.parse_args(["--hook-event", "PreCompact"])
    # Should NOT raise — hook mode makes tmux args optional.
    _validate_args(parser, args)


# ---------------------------------------------------------------------------
# g-003 + g-006 — _dispatch uses cwd-relative handoff dir + adapter.command()
# ---------------------------------------------------------------------------


class _FakeAdapter:
    """Records the logical commands the dispatcher asked for."""

    name = "fake"

    def __init__(self) -> None:
        self.requested: list[str] = []

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        self.requested.append(logical)
        return {
            "clear": "/FAKE_CLEAR",
            "compact": "/FAKE_COMPACT",
            "handoff": "/FAKE_HANDOFF",
        }[logical]

    def prompt_marker(self) -> str:
        # Empty marker → _dispatch's _prompt_ok guard short-circuits to True
        # without shelling out to tmux (these unit tests run without a tmux
        # server). Matches the Protocol signature; a real adapter returns a
        # non-empty prompt marker.
        return ""


def _make_valid_handoff(handoff_dir: Path) -> Path:
    """Write a structurally-valid handoff file under *handoff_dir*."""
    handoff_dir.mkdir(parents=True, exist_ok=True)
    f = handoff_dir / f"{time.strftime('%Y-%m-%d')}-test.md"
    f.write_text(
        "# Handoff test\n\n"
        "## Status\nIn progress.\n\n"
        "## Next step\nKeep going.\n\n"
        + ("x" * 300)
        + "\n",
        encoding="utf-8",
    )
    return f


def test_g006_dispatch_uses_adapter_command_for_compact(tmp_path):
    """_dispatch('compact') routes through adapter.command('compact')."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()
    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    with patch("autospec_context_monitor.__main__.inject", _fake_inject):
        _dispatch(
            Action("compact"),
            "test-sess",
            tmp_path / "log",
            harness="fake",
            cwd=str(tmp_path),
            pct=0.55,
            adapter=adapter,
        )

    assert "compact" in adapter.requested
    assert injects == ["/FAKE_COMPACT"]


def test_g006_dispatch_uses_adapter_command_for_clear(tmp_path):
    """_dispatch('clear') routes through adapter.command('clear') (e.g. /new for codex)."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()
    _make_valid_handoff(tmp_path / ".turbo" / "handoff")

    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    with patch("autospec_context_monitor.__main__.inject", _fake_inject):
        with patch("autospec_context_monitor.__main__.wait_for_cancel", return_value=False):
            _dispatch(
                Action("clear"),
                "test-sess",
                tmp_path / "log",
                harness="fake",
                cwd=str(tmp_path),
                pct=0.85,
                adapter=adapter,
            )

    assert "clear" in adapter.requested
    assert injects == ["/FAKE_CLEAR"]


def test_g006_dispatch_uses_adapter_command_for_handoff(tmp_path):
    """_dispatch('handoff') routes through adapter.command('handoff')."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()
    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    # wait_for_handoff is called after the inject; pretend a handoff appears
    # immediately so the test doesn't block 180s.
    fake_path = _make_valid_handoff(tmp_path / ".turbo" / "handoff")

    with patch("autospec_context_monitor.__main__.inject", _fake_inject):
        with patch(
            "autospec_context_monitor.__main__.wait_for_handoff",
            return_value=fake_path,
        ):
            _dispatch(
                Action("handoff"),
                "test-sess",
                tmp_path / "log",
                harness="fake",
                cwd=str(tmp_path),
                pct=0.85,
                adapter=adapter,
            )

    assert "handoff" in adapter.requested
    assert injects == ["/FAKE_HANDOFF"]


def test_g003_handoff_path_resolves_under_cwd(tmp_path):
    """_dispatch('clear') reads handoff files from <cwd>/.turbo/handoff, not ~/.turbo/handoff."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()
    # Valid handoff under CWD (correct location per spec)
    _make_valid_handoff(tmp_path / ".turbo" / "handoff")

    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    # Make ~/.turbo/handoff a *non-existent* path so the old behaviour would
    # vacuously pass (no files found). The fix means we read from cwd instead
    # and DO find the valid handoff there.
    bogus_home = tmp_path / "fake-home"
    bogus_home.mkdir()

    with patch("autospec_context_monitor.__main__.Path.home", return_value=bogus_home):
        with patch("autospec_context_monitor.__main__.inject", _fake_inject):
            with patch("autospec_context_monitor.__main__.wait_for_cancel", return_value=False):
                result = _dispatch(
                    Action("clear"),
                    "test-sess",
                    tmp_path / "log",
                    harness="fake",
                    cwd=str(tmp_path),
                    pct=0.85,
                    adapter=adapter,
                )

    assert result is False, "clear with valid cwd-rooted handoff must not abort"
    assert injects == ["/FAKE_CLEAR"]


def test_g003_resume_reads_cwd_handoff(tmp_path):
    """_dispatch('resume') sources the latest handoff file from <cwd>/.turbo/handoff."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()
    fpath = _make_valid_handoff(tmp_path / ".turbo" / "handoff")

    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    bogus_home = tmp_path / "fake-home"
    bogus_home.mkdir()

    with patch("autospec_context_monitor.__main__.Path.home", return_value=bogus_home):
        with patch("autospec_context_monitor.__main__.inject", _fake_inject):
            _dispatch(
                Action("resume"),
                "test-sess",
                tmp_path / "log",
                harness="fake",
                cwd=str(tmp_path),
                pct=0.30,
                adapter=adapter,
            )

    # Resume injects the cwd-rooted handoff filename, not the home one.
    assert len(injects) == 1
    assert str(fpath) in injects[0], (
        f"Resume prompt must reference the cwd handoff file. Got: {injects[0]!r}"
    )


# ---------------------------------------------------------------------------
# g-004 — handoff wait is non-blocking (issue #898 Part 2, FIX B)
#
# The handoff wait NO LONGER blocks inside _dispatch (that froze the whole poll
# loop for up to 180s). _dispatch('handoff') now only injects /create-handoff
# and returns immediately; the wait is owned by the poll loop via
# _PendingHandoff / _advance_pending_handoff, which polls non-blockingly and
# dispatches the deferred clear+resume once the file appears (or reverts to
# COMPACTED on deadline). These tests lock in that contract.
# ---------------------------------------------------------------------------


def test_g004_dispatch_handoff_injects_without_blocking(tmp_path):
    """Action('handoff') injects /create-handoff and returns WITHOUT calling
    wait_for_handoff (no 180s block inside _dispatch)."""
    from autospec_context_monitor.__main__ import _dispatch

    adapter = _FakeAdapter()

    wait_calls: list = []

    def _fake_wait(*a, **_kw):
        wait_calls.append(a)
        raise AssertionError("wait_for_handoff must NOT be called from _dispatch")

    injects: list[str] = []

    def _fake_inject(_sess, text, **_kw):
        injects.append(text)

    with patch("autospec_context_monitor.__main__.inject", _fake_inject):
        with patch("autospec_context_monitor.__main__.wait_for_handoff", _fake_wait):
            result = _dispatch(
                Action("handoff"),
                "test-sess",
                tmp_path / "log",
                harness="fake",
                cwd=str(tmp_path),
                pct=0.85,
                adapter=adapter,
            )

    assert "handoff" in adapter.requested
    assert injects == ["/FAKE_HANDOFF"]
    assert result is False, "_dispatch('handoff') must not signal cancel by itself"
    assert not wait_calls, "the 180s blocking wait must not run inside _dispatch"


def test_g004_advance_pending_handoff_completes_on_file(tmp_path):
    """Once the handoff file appears, _advance_pending_handoff dispatches the
    deferred clear+resume and reports ('done', not-canceled)."""
    from autospec_context_monitor.__main__ import _advance_pending_handoff, _PendingHandoff

    adapter = _FakeAdapter()
    _make_valid_handoff(tmp_path / ".turbo" / "handoff")

    injects: list[str] = []

    pending = _PendingHandoff(
        since=time.time() - 5,  # handoff written after this
        deadline=time.monotonic() + 180.0,
        rest=[Action("clear"), Action("resume")],
    )

    with patch("autospec_context_monitor.__main__.inject", lambda _s, t, **_k: injects.append(t)):
        with patch("autospec_context_monitor.__main__.wait_for_cancel", return_value=False):
            status, canceled = _advance_pending_handoff(
                pending, tmp_path, tmp_path / "log",
                harness="fake", tmux_session="test-sess", cwd=str(tmp_path),
                pct=0.85, adapter=adapter,
            )

    assert status == "done"
    assert canceled is False
    assert "/FAKE_CLEAR" in injects  # deferred clear ran
    assert any("continue from where" in t.lower() for t in injects)  # resume ran


def test_g004_advance_pending_handoff_aborts_on_deadline(tmp_path):
    """Past the deadline with no handoff file, _advance_pending_handoff reports
    ('timeout', canceled=True) so the loop reverts state to COMPACTED."""
    from autospec_context_monitor.__main__ import _advance_pending_handoff, _PendingHandoff

    adapter = _FakeAdapter()  # no handoff file written

    pending = _PendingHandoff(
        since=time.time(),
        deadline=100.0,
        rest=[Action("clear"), Action("resume")],
    )

    status, canceled = _advance_pending_handoff(
        pending, tmp_path, tmp_path / "log",
        harness="fake", tmux_session="test-sess", cwd=str(tmp_path),
        pct=0.85, adapter=adapter, now=101.0,  # past deadline
    )

    assert status == "timeout"
    assert canceled is True, (
        "deadline-exceeded handoff must signal cancel so the main loop reverts "
        "state to COMPACTED instead of firing /clear."
    )


# ---------------------------------------------------------------------------
# g-005 — engine NORMAL → ROLLED shortcut is removed
# ---------------------------------------------------------------------------


def test_g005_engine_normal_above_80_emits_compact_only():
    """NORMAL + pct>=0.80 → emit [compact] and transition to COMPACTED, not ROLLED.

    Spec §Threshold state machine: NORMAL → COMPACTED → ROLLED is the only path.
    """
    e = Engine()
    usage = Usage(used_tokens=850, max_tokens=1000, model="test", estimated=False)
    actions = e.classify(usage)

    assert e.state is State.COMPACTED, (
        f"Engine must transition NORMAL → COMPACTED (not ROLLED) on pct>=0.80. "
        f"Got: {e.state}"
    )
    assert [a.kind for a in actions] == ["compact"], (
        f"Engine must emit [compact] only when transitioning NORMAL→COMPACTED at high pct. "
        f"Got: {[a.kind for a in actions]}"
    )


def test_g005_engine_compact_then_rollover_two_ticks():
    """Two ticks at pct>=0.80 yield [compact] then [handoff, clear, resume]."""
    e = Engine()
    high = Usage(used_tokens=850, max_tokens=1000, model="test", estimated=False)

    first = e.classify(high)
    assert [a.kind for a in first] == ["compact"]
    assert e.state is State.COMPACTED

    second = e.classify(high)
    assert [a.kind for a in second] == ["handoff", "clear", "resume"]
    assert e.state is State.ROLLED


def test_g005_engine_normal_jump_to_50_still_compacts():
    """NORMAL + pct in [0.50, 0.80) still emits [compact] (unchanged path)."""
    e = Engine()
    usage = Usage(used_tokens=600, max_tokens=1000, model="test", estimated=False)
    actions = e.classify(usage)
    assert e.state is State.COMPACTED
    assert [a.kind for a in actions] == ["compact"]
