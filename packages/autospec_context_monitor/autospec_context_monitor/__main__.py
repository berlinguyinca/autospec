"""python -m autospec_context_monitor — context-window monitor daemon.

Two invocation modes:

**tmux-driven (default):**
::

    python -m autospec_context_monitor \\
        --tmux-session my-cc \\
        --harness claude \\
        --cwd /path/to/project \\
        --started-at 1717200000

**Claude PreCompact / SessionStart hook (no tmux):**
::

    python -m autospec_context_monitor --hook-event PreCompact

The daemon (tmux mode):
1. Writes a PID file at ``~/.autospec/monitors/<session>.pid``.
2. Appends JSON-lines to ``~/.autospec/monitors/<session>.log``.
3. Polls every 15 s; on each tick:
   - Checks ``~/.autospec/no-auto-rollover.flag`` (kill-switch).
   - Checks the tmux session is still alive.
   - Reads Usage via the selected adapter; runs Engine.classify(); dispatches
     Actions via inject. A pending ``/create-handoff`` is awaited
     non-blockingly across ticks (see ``_PendingHandoff``) so the poll cadence
     is never frozen.
4. Removes the PID file on normal exit or SIGTERM.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from .adapters.base import TranscriptNotFoundError, Usage
from .engine import Action, Engine, State
from .handoff import (
    HandoffTimeoutError,
    check_handoff,
    validate_handoff,
    wait_for_handoff,
)

# ``wait_for_handoff`` / ``HandoffTimeoutError`` are no longer called from the
# dispatch path (the handoff wait is now non-blocking, owned by the poll loop
# via ``_PendingHandoff`` / ``_advance_pending_handoff``). They remain imported
# as part of this module's public surface: external callers and existing tests
# reference ``autospec_context_monitor.__main__.wait_for_handoff``.
_ = (HandoffTimeoutError, wait_for_handoff)
from .injector import SessionGone, inject, session_exists, wait_for_cancel, wait_for_prompt
from . import stats as _stats

_POLL_INTERVAL = 15  # seconds
# Wall-clock budget for the harness to write a handoff file after /create-handoff
# is injected. Previously consumed by a single 180s blocking wait_for_handoff;
# now spread across poll ticks so the loop stays responsive (kill-switch /
# session-alive / usage keep being checked while the handoff is pending).
_HANDOFF_DEADLINE = 180.0  # seconds
_KILL_SWITCH = Path.home() / ".autospec" / "no-auto-rollover.flag"
_MONITORS_DIR = Path.home() / ".autospec" / "monitors"

# Hook-mode constants
_HOOK_EVENTS = ("PreCompact", "SessionStart")
# Window (seconds) used to judge a handoff "fresh" when no explicit session
# start timestamp is available — a handoff written within this many seconds of
# the transcript's last activity is treated as belonging to the current session.
_FRESH_WINDOW = 6 * 60 * 60  # 6 hours


# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

def _log(logf: Path, record: dict) -> None:
    """Append a JSON-line record to *logf* with an ISO timestamp."""
    record.setdefault("ts", time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
    with logf.open("a") as fh:
        fh.write(json.dumps(record) + "\n")


# ---------------------------------------------------------------------------
# Adapter loader
# ---------------------------------------------------------------------------

def _load_adapter(harness: str):
    """Return a concrete HarnessAdapter for *harness*."""
    if harness == "claude":
        from .adapters.claude import ClaudeAdapter
        return ClaudeAdapter()
    if harness == "codex":
        from .adapters.codex import CodexAdapter
        return CodexAdapter()
    if harness == "opencode":
        from .adapters.opencode import OpenCodeAdapter
        return OpenCodeAdapter()
    if harness == "claude_hook":
        from .adapters.claude_hook import ClaudeHookAdapter
        return ClaudeHookAdapter()
    raise ValueError(f"Unknown harness: {harness!r}")


# ---------------------------------------------------------------------------
# Action dispatcher
# ---------------------------------------------------------------------------

def _dispatch(
    action: Action,
    tmux_session: str,
    logf: Path,
    harness: str = "",
    cwd: str = "",
    pct: float = 0.0,
    adapter=None,
) -> bool:
    """Execute *action* by injecting into the tmux session.

    Every slash injection is routed through ``adapter.command(...)`` so each
    harness can map logical names to its own slash commands (e.g. Codex maps
    ``"clear"`` → ``/new``, OpenCode maps ``"handoff"`` to a prose prompt).

    Handoff handling:
    - ``Action('handoff')`` injects ``adapter.command('handoff')`` and returns
      immediately. It does NOT block waiting for the handoff file — that wait
      is owned by the main poll loop via :class:`_PendingHandoff` /
      :func:`_advance_pending_handoff`, which polls non-blockingly each tick so
      the loop stays responsive. The deferred ``clear`` + ``resume`` actions
      are dispatched once the handoff file is observed.
    - ``Action('clear')`` validates the most recent handoff file under
      ``<cwd>/.turbo/handoff`` (per spec) before injecting.

    Args:
        action:        The Action to dispatch.
        tmux_session:  Tmux session name to inject into.
        logf:          Path to the daemon log file.
        harness:       Harness name (claude/codex/opencode).
        cwd:           Working directory of the harness; doubles as the repo
                       root for handoff-file discovery.
        pct:           Current context-usage fraction (for stats).
        adapter:       HarnessAdapter instance. When ``None`` (legacy callers
                       and tests), falls back to Claude-style literals so the
                       function remains backward-compatible.

    Returns:
        ``True`` if the action was canceled (user pressed Esc during the
        clear cancel window, or the handoff failed validation); ``False``
        otherwise. The handoff *timeout* path no longer flows through here —
        it is handled by :func:`_advance_pending_handoff`.
    """
    if action.kind == "noop":
        _log(logf, {"event": "noop", "payload": action.payload})
        return False

    # Resolve handoff dir under the *repo root* (cwd), not $HOME. The spec and
    # the /create-handoff skill both write to <repo>/.turbo/handoff/.
    handoff_dir = (
        Path(cwd) / ".turbo" / "handoff"
        if cwd
        else Path.home() / ".turbo" / "handoff"
    )

    # UX overlays: show a brief tmux message so the operator sees rollover is in progress.
    _OVERLAY_MSG = {
        "compact": "[autospec] compacting context…",
        "handoff": "[autospec] writing handoff…",
        "clear": "[autospec] clearing session…",
        "resume": "[autospec] resuming…",
    }
    overlay_msg = _OVERLAY_MSG.get(action.kind)
    if overlay_msg and tmux_session:
        subprocess.run(["tmux", "display-message", "-d", "3000", overlay_msg], check=False)

    # Prompt-ready guard helper: returns the marker and whether the prompt is ready.
    def _prompt_ok(kind: str) -> bool:
        if not tmux_session or adapter is None:
            return True  # no guard when tmux or adapter absent (e.g. hook mode / tests)
        marker = adapter.prompt_marker()
        if not marker:
            return True
        ready = wait_for_prompt(tmux_session, marker)
        if not ready:
            _log(logf, {"event": "prompt_not_ready", "kind": kind, "marker": marker})
        return ready

    if action.kind == "compact":
        cmd = adapter.command("compact") if adapter is not None else "/compact"
        _log(logf, {"event": "inject", "kind": "compact", "cmd": cmd})
        if not _prompt_ok("compact"):
            return False
        inject(tmux_session, cmd)
        _stats.record(
            "compact_fired",
            harness=harness,
            tmux_session=tmux_session,
            pct=round(pct, 4),
            cwd=cwd,
        )
        return False

    if action.kind == "handoff":
        # Inject /create-handoff and return immediately. The blocking wait for
        # the handoff file to appear is NOT done here — it would freeze the
        # entire poll loop for up to 180s. Instead the main loop carries a
        # deadline-based "pending handoff" state and polls non-blockingly each
        # tick (see _check_pending_handoff and the loop). Injection of the
        # subsequent /clear + resume is deferred until the file is observed.
        cmd = adapter.command("handoff") if adapter is not None else "/create-handoff"
        _log(logf, {"event": "inject", "kind": "handoff", "cmd": cmd})
        if not _prompt_ok("handoff"):
            return False
        inject(tmux_session, cmd)
        return False

    if action.kind == "clear":
        # --- Handoff validation gate (must run BEFORE cancel window per spec) ---
        hfiles = (
            sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime)
            if handoff_dir.exists()
            else []
        )
        if not hfiles:
            # No handoff file found — abort rollover before showing cancel window.
            msg = "autospec: no handoff file found — rollover aborted"
            _log(
                logf,
                {
                    "event": "no_handoff_file",
                    "kind": "clear",
                    "msg": msg,
                },
            )
            _stats.record(
                "handoff_invalid",
                harness=harness,
                tmux_session=tmux_session,
                pct=round(pct, 4),
                cwd=cwd,
            )
            subprocess.run(["tmux", "display-message", msg], check=False)
            subprocess.run(
                ["tmux", "send-keys", "-t", tmux_session, "-l", "\a"],
                check=False,
            )
            return True

        ok, missing = validate_handoff(hfiles[-1])
        if not ok:
            msg = (
                f"autospec: handoff invalid (missing: {', '.join(missing)}) "
                "— rollover aborted"
            )
            _log(
                logf,
                {
                    "event": "handoff invalid: missing",
                    "missing": missing,
                    "kind": "clear",
                },
            )
            _stats.record(
                "handoff_invalid",
                harness=harness,
                tmux_session=tmux_session,
                pct=round(pct, 4),
                cwd=cwd,
            )
            subprocess.run(["tmux", "display-message", msg], check=False)
            subprocess.run(
                ["tmux", "send-keys", "-t", tmux_session, "-l", "\a"],
                check=False,
            )
            return True

        # --- Cancel window (only reached when handoff validation passes) ---
        _log(logf, {"event": "cancel_window_start", "kind": "clear"})
        if wait_for_cancel(tmux_session):
            _log(logf, {"event": "rollover canceled by user", "kind": "clear"})
            _stats.record(
                "rollover_aborted",
                harness=harness,
                tmux_session=tmux_session,
                pct=round(pct, 4),
                cwd=cwd,
            )
            return True

        cmd = adapter.command("clear") if adapter is not None else "/clear"
        _log(logf, {"event": "inject", "kind": "clear", "cmd": cmd})
        if not _prompt_ok("clear"):
            return False
        inject(tmux_session, cmd)
        _stats.record(
            "rollover_fired",
            harness=harness,
            tmux_session=tmux_session,
            pct=round(pct, 4),
            cwd=cwd,
        )
        return False

    if action.kind == "resume":
        # Find the latest handoff file under the repo root and inject a
        # resume prompt referencing it.
        files = (
            sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime)
            if handoff_dir.exists()
            else []
        )
        if files:
            latest = files[-1]
            resume_text = (
                f"Read {latest} and continue from where the previous session left off."
            )
        else:
            latest = None
            resume_text = "Continue from where the previous session left off."
        _log(
            logf,
            {
                "event": "inject",
                "kind": "resume",
                "file": str(latest) if latest else None,
            },
        )
        if not _prompt_ok("resume"):
            return False
        inject(tmux_session, resume_text)
        return False

    _log(logf, {"event": "unknown_action", "kind": action.kind})
    return False


# ---------------------------------------------------------------------------
# Argument parser (exposed for testing)
# ---------------------------------------------------------------------------

def _build_parser() -> argparse.ArgumentParser:
    """Build the argparse parser for the monitor daemon.

    Two invocation modes are supported:

    1. **tmux mode** — requires ``--tmux-session``, ``--harness``, ``--cwd``.
    2. **hook mode** — ``--hook-event {PreCompact,SessionStart}`` (used by
       Claude Code's native PreCompact/SessionStart hooks registered via
       ``install.sh --hook-mode claude``). In hook mode, ``--tmux-session``
       and friends become optional and the daemon dispatches via the
       ``claude_hook`` adapter (no tmux required).
    """
    parser = argparse.ArgumentParser(
        prog="python -m autospec_context_monitor",
        description="Context-window monitor daemon for the autospec workflow suite.",
    )
    parser.add_argument(
        "--hook-event",
        choices=list(_HOOK_EVENTS),
        default=None,
        help="Claude Code native-hook event name. When set, --tmux-session is optional.",
    )
    parser.add_argument(
        "--tmux-session",
        default=None,
        help="Tmux session name to monitor (required unless --hook-event is set).",
    )
    parser.add_argument(
        "--harness",
        choices=["claude", "codex", "opencode"],
        default=None,
        help="AI harness running inside the tmux session.",
    )
    parser.add_argument(
        "--cwd",
        default=None,
        help="Working directory of the harness process.",
    )
    parser.add_argument(
        "--started-at",
        type=float,
        default=None,
        help="Unix timestamp when the harness session started (for transcript discovery).",
    )
    return parser


def _validate_args(parser: argparse.ArgumentParser, args: argparse.Namespace) -> None:
    """Enforce mode-dependent required arguments.

    - Without ``--hook-event``: ``--tmux-session``, ``--harness``, ``--cwd``
      are required (legacy tmux-driven invocation).
    - With ``--hook-event``: those flags become optional; the hook adapter
      derives what it needs from the environment.
    """
    if args.hook_event is None:
        missing = [
            name
            for name, val in (
                ("--tmux-session", args.tmux_session),
                ("--harness", args.harness),
                ("--cwd", args.cwd),
            )
            if not val
        ]
        if missing:
            parser.error(
                "the following arguments are required when --hook-event is not set: "
                + ", ".join(missing)
            )


class _PendingHandoff:
    """Deadline-based carrier for an in-flight ``/create-handoff`` wait.

    Created when the daemon injects ``/create-handoff`` (the ``handoff``
    action) and consulted on every subsequent poll tick. It replaces the old
    single 180s blocking :func:`wait_for_handoff` call so the poll loop keeps
    checking the kill-switch / session-alive / usage while the handoff is
    pending.

    Attributes:
        since:    Timestamp captured just before ``/create-handoff`` was
                  injected; only handoff files newer than this count.
        deadline: ``time.monotonic()`` value past which the wait is abandoned.
        rest:     The remaining actions to run once the handoff appears
                  (typically ``[clear, resume]``).
    """

    __slots__ = ("since", "deadline", "rest")

    def __init__(self, since: float, deadline: float, rest: list[Action]) -> None:
        self.since = since
        self.deadline = deadline
        self.rest = rest


def _advance_pending_handoff(
    pending: _PendingHandoff,
    repo_root: Path,
    logf: Path,
    *,
    harness: str,
    tmux_session: str,
    cwd: str,
    pct: float,
    adapter,
    now: float | None = None,
) -> tuple[str, bool]:
    """Advance an in-flight handoff wait by exactly one non-blocking poll.

    Performs a single :func:`check_handoff` scan (never blocks). When the
    handoff file has appeared, dispatches the deferred actions (``clear`` then
    ``resume``); when the deadline has passed without a file, reports a timeout.

    Args:
        pending: The :class:`_PendingHandoff` carrier to advance.
        repo_root: Repo root containing ``.turbo/handoff``.
        logf: Daemon log path.
        now: Injectable ``time.monotonic()`` value (for fast, deterministic
             tests). Defaults to the real clock.

    Returns:
        A ``(status, canceled)`` tuple. ``status`` is one of:
        - ``"pending"`` — no file yet and deadline not reached; keep waiting.
        - ``"done"``    — handoff observed; deferred actions dispatched.
        - ``"timeout"`` — deadline exceeded with no handoff; rollover aborted.
        ``canceled`` is ``True`` when the loop should revert engine state to
        COMPACTED (on timeout, or if a deferred action canceled).
    """
    path = check_handoff(repo_root, pending.since)
    if path is not None:
        _log(logf, {"event": "handoff_written", "path": str(path)})
        canceled = False
        for action in pending.rest:
            if _dispatch(
                action,
                tmux_session,
                logf,
                harness=harness,
                cwd=cwd,
                pct=pct,
                adapter=adapter,
            ):
                canceled = True
                break
        return "done", canceled

    clock = time.monotonic() if now is None else now
    if clock >= pending.deadline:
        _log(
            logf,
            {
                "event": "handoff_timeout",
                "err": (
                    f"no handoff under {repo_root / '.turbo' / 'handoff'} "
                    f"with mtime > {pending.since:.3f} before deadline"
                ),
            },
        )
        _stats.record(
            "handoff_timeout",
            harness=harness,
            tmux_session=tmux_session,
            pct=round(pct, 4),
            cwd=cwd,
        )
        return "timeout", True

    return "pending", False


# ---------------------------------------------------------------------------
# Hook-mode entry point
# ---------------------------------------------------------------------------

def _resolve_session_start(args: argparse.Namespace, transcript: "Path | None") -> float:
    """Resolve a best-effort "session start" cutoff for fresh-handoff checks.

    Preference order:
    1. ``args.started_at`` (the harness-supplied session start timestamp).
    2. The transcript's last-modified time minus a recent window
       (``_FRESH_WINDOW``) — the transcript is rewritten throughout the
       session, so its mtime tracks recent activity; subtracting a window
       lets us treat a handoff written shortly before this PreCompact as
       "fresh" without admitting one from a prior, long-idle session.
    3. ``time.time() - _FRESH_WINDOW`` when neither is available.

    A handoff is considered *fresh* iff its mtime is strictly greater than the
    returned cutoff.
    """
    if args.started_at is not None:
        try:
            return float(args.started_at)
        except (TypeError, ValueError):
            pass
    if transcript is not None:
        try:
            return transcript.stat().st_mtime - _FRESH_WINDOW
        except OSError:
            pass
    return time.time() - _FRESH_WINDOW


def _hook_daemon_running(args: argparse.Namespace) -> bool:
    """Best-effort check for a live tmux-daemon monitor for this session.

    True auto-rollover (handoff -> clear -> resume) is performed *only* by the
    tmux-daemon path, which writes a ``<session>.pid`` file under
    :data:`_MONITORS_DIR`. Hook mode is notification-only, so we look for any
    live (PID still running) monitor pidfile and, when none is found, warn that
    the hook cannot actually roll over on its own.
    """
    if not _MONITORS_DIR.exists():
        return False
    for pidf in _MONITORS_DIR.glob("*.pid"):
        try:
            pid = int(pidf.read_text().strip())
        except (OSError, ValueError):
            continue
        try:
            os.kill(pid, 0)  # signal 0: existence check, no signal delivered
        except OSError:
            continue
        return True
    return False


def _run_hook_mode(args: argparse.Namespace) -> None:
    """Handle Claude Code PreCompact / SessionStart hook invocations.

    **Notification-only by design.** A PreCompact hook runs as a short-lived
    Python subprocess *inside* Claude Code with no tmux pane to drive and no
    way to summarize the conversation — it cannot run ``/create-handoff``,
    cannot ``/clear``, and cannot ``/resume``. It can only *observe* usage and
    surface a desktop notification. Real handoff->clear->resume orchestration
    requires the tmux-daemon mode (the ``_dispatch`` path), which injects slash
    commands into a live pane.

    So at ``PreCompact`` this function:
    - Reads usage from the transcript (logged for diagnostics).
    - Surfaces a desktop notification that points at a *fresh* handoff (one
      written after this session started) when one exists, or honestly says
      "no fresh handoff — run /create-handoff before compaction" when the only
      handoff on disk predates the session (a stale handoff must never be
      presented as if it captured the current work).
    - Warns (log + notification note) when hook mode is the *only* thing wired
      (no live tmux daemon for this machine): auto-rollover will not happen and
      the user must create the handoff manually.

    ``SessionStart`` is currently a no-op (reserved for resume hints in a
    future revision).

    All output goes to ``~/.autospec/monitors/hook-<event>.log`` so the daemon
    never pollutes the Claude Code transcript.  Failures are logged and
    swallowed: the hook must never break Claude Code's compaction.
    """
    _MONITORS_DIR.mkdir(parents=True, exist_ok=True)
    logf = _MONITORS_DIR / f"hook-{args.hook_event}.log"
    _log(logf, {"event": "hook_start", "hook_event": args.hook_event, "pid": os.getpid()})

    if args.hook_event == "SessionStart":
        # Reserved for future use (e.g. surfacing the latest handoff to the
        # newly-started session). For now, return without side effects.
        _log(logf, {"event": "hook_noop", "hook_event": "SessionStart"})
        return

    # PreCompact: best-effort, notification-only path.
    try:
        from .adapters.claude_hook import ClaudeHookAdapter

        adapter = ClaudeHookAdapter()
        cwd = args.cwd or os.getcwd()
        hint = {
            "cwd": cwd,
            "tmux_session": args.tmux_session,
            "started_at": args.started_at,
        }
        transcript = adapter.find_transcript(hint)
        usage = adapter.read_usage(transcript)
        _log(
            logf,
            {
                "event": "usage",
                "used": usage.used_tokens,
                "max": usage.max_tokens,
                "pct": round(usage.pct, 4),
                "model": usage.model,
            },
        )

        # Resolve the *fresh* handoff under <cwd>/.turbo/handoff: only a file
        # newer than the session-start cutoff may be surfaced. Pointing at a
        # stale handoff from a prior session would mislead the user into
        # compacting on top of obsolete context.
        cutoff = _resolve_session_start(args, transcript)
        handoff_dir = Path(cwd) / ".turbo" / "handoff"
        files = (
            sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime)
            if handoff_dir.exists()
            else []
        )
        fresh = [p for p in files if p.stat().st_mtime > cutoff]
        latest_fresh = fresh[-1] if fresh else None

        # Warn when nothing can actually perform the rollover (no live daemon).
        daemon_running = _hook_daemon_running(args)
        if not daemon_running:
            _log(
                logf,
                {
                    "event": "hook_only_no_daemon",
                    "note": (
                        "hook mode is notification-only; true auto-rollover "
                        "(handoff->clear->resume) requires the tmux/daemon mode"
                    ),
                },
            )

        if latest_fresh is not None:
            ClaudeHookAdapter.notify_clear_needed(handoff_path=latest_fresh)
            _log(
                logf,
                {
                    "event": "hook_notify",
                    "handoff": str(latest_fresh),
                    "fresh": True,
                    "daemon_running": daemon_running,
                },
            )
        else:
            # No fresh handoff — be honest rather than pointing at a stale file.
            note = "No fresh handoff — run /create-handoff before compaction."
            if not daemon_running:
                note += " (hook mode is notification-only; no daemon to roll over)"
            ClaudeHookAdapter.notify_no_fresh_handoff(note)
            _log(
                logf,
                {
                    "event": "hook_notify_no_fresh",
                    "stale_present": bool(files),
                    "stale_latest": str(files[-1]) if files else None,
                    "daemon_running": daemon_running,
                },
            )
    except Exception as exc:  # noqa: BLE001
        _log(
            logf,
            {"event": "hook_error", "err": str(exc), "type": type(exc).__name__},
        )


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> None:
    """Entry point for the context-window monitor daemon."""
    parser = _build_parser()
    args = parser.parse_args(argv)
    _validate_args(parser, args)

    # Hook-mode invocations branch out before the tmux poll loop.
    if args.hook_event is not None:
        _run_hook_mode(args)
        return

    # ---- tmux-mode setup ----
    engine = Engine()
    hint: dict = {
        "cwd": args.cwd,
        "tmux_session": args.tmux_session,
        "started_at": args.started_at,
    }

    _MONITORS_DIR.mkdir(parents=True, exist_ok=True)
    pidf = _MONITORS_DIR / f"{args.tmux_session}.pid"
    logf = _MONITORS_DIR / f"{args.tmux_session}.log"

    pidf.write_text(str(os.getpid()))
    _log(logf, {
        "event": "start",
        "pid": os.getpid(),
        "tmux_session": args.tmux_session,
        "harness": args.harness,
        "cwd": args.cwd,
        "log": str(logf),
    })
    print(f"autospec-context-monitor: log at {logf}", flush=True)

    def _sigterm_handler(*_):
        _log(logf, {"event": "sigterm"})
        pidf.unlink(missing_ok=True)
        sys.exit(0)

    signal.signal(signal.SIGTERM, _sigterm_handler)

    # ---- poll loop ----
    # Track the transcript path seen when ROLLED state was entered so that
    # a new transcript (new path) triggers an immediate reset to NORMAL.
    last_transcript: Path | None = None
    # In-flight handoff wait, advanced one non-blocking tick at a time so the
    # poll cadence (kill-switch / session-alive / usage) is never frozen.
    pending_handoff: _PendingHandoff | None = None
    repo_root = Path(args.cwd) if args.cwd else Path.cwd()

    def _revert_to_compacted() -> None:
        engine._state = State.COMPACTED  # noqa: SLF001
        _log(logf, {"event": "state_reverted", "state": "COMPACTED"})

    try:
        # Load adapter inside the try block so PID cleanup runs even if
        # _load_adapter raises ImportError or ValueError.
        adapter = _load_adapter(args.harness)
        while True:
            if _KILL_SWITCH.exists():
                _log(logf, {"event": "kill_switch", "reason": "no-auto-rollover.flag present"})
                break

            if not session_exists(args.tmux_session):
                _log(logf, {"event": "session_gone", "tmux_session": args.tmux_session})
                break

            # Advance any in-flight handoff wait FIRST (non-blocking). This is
            # what keeps the loop responsive: instead of one 180s blocking
            # wait_for_handoff inside _dispatch, we poll once per tick and fall
            # through to sleep — so the kill-switch / session checks above keep
            # running while the handoff is pending.
            if pending_handoff is not None:
                status, canceled = _advance_pending_handoff(
                    pending_handoff,
                    repo_root,
                    logf,
                    harness=args.harness,
                    tmux_session=args.tmux_session,
                    cwd=args.cwd,
                    pct=0.0,
                    adapter=adapter,
                )
                if status != "pending":
                    pending_handoff = None
                    if canceled:
                        # Handoff timed out (or a deferred clear/resume was
                        # canceled) — revert so the engine re-fires on the next
                        # 80% climb rather than waiting for a new transcript.
                        _revert_to_compacted()
                # While a handoff is pending we skip classify/dispatch for this
                # tick (the rollover sequence is mid-flight); just sleep and
                # re-check liveness next tick.
                time.sleep(_POLL_INTERVAL)
                continue

            try:
                transcript = adapter.find_transcript(hint)

                # ROLLED → NORMAL on new transcript detection (g-016).
                if engine.state is State.ROLLED and transcript and transcript != last_transcript:
                    engine.reset()
                    last_transcript = transcript
                    import logging as _logging
                    _logging.getLogger(__name__).info(
                        "[autospec] new transcript detected, resetting to NORMAL"
                    )
                    _log(logf, {"event": "rolled_reset", "transcript": str(transcript)})

                usage: Usage = adapter.read_usage(transcript)
                _log(logf, {
                    "event": "usage",
                    "used": usage.used_tokens,
                    "max": usage.max_tokens,
                    "pct": round(usage.used_tokens / usage.max_tokens, 4),
                    "model": usage.model,
                })
                pct = round(usage.used_tokens / usage.max_tokens, 4)
                actions = engine.classify(usage)
                for idx, action in enumerate(actions):
                    if action.kind == "handoff":
                        # Inject /create-handoff, then DON'T block: hand the
                        # remaining actions ([clear, resume]) to a deadline-based
                        # pending state the loop advances on later ticks.
                        since = time.time()
                        canceled = _dispatch(
                            action,
                            args.tmux_session,
                            logf,
                            harness=args.harness,
                            cwd=args.cwd,
                            pct=pct,
                            adapter=adapter,
                        )
                        if canceled:
                            _revert_to_compacted()
                            break
                        pending_handoff = _PendingHandoff(
                            since=since,
                            deadline=time.monotonic() + _HANDOFF_DEADLINE,
                            rest=list(actions[idx + 1:]),
                        )
                        _log(
                            logf,
                            {
                                "event": "handoff_pending",
                                "deadline_s": _HANDOFF_DEADLINE,
                                "rest": [a.kind for a in pending_handoff.rest],
                            },
                        )
                        # Stop dispatching remaining actions now; they run once
                        # the handoff file is observed.
                        break

                    canceled = _dispatch(
                        action,
                        args.tmux_session,
                        logf,
                        harness=args.harness,
                        cwd=args.cwd,
                        pct=pct,
                        adapter=adapter,
                    )
                    if canceled:
                        # User pressed Esc during cancel window or handoff
                        # validation failed — revert state so the engine
                        # re-fires the rollover on the next 80% climb rather
                        # than waiting for a new transcript.
                        _revert_to_compacted()
                        break

                # Update last_transcript after each successful tick so the
                # ROLLED→NORMAL check compares against a stable baseline.
                last_transcript = transcript
            except TranscriptNotFoundError as exc:
                _log(logf, {"event": "no_transcript", "err": str(exc)})
            except SessionGone:
                _log(logf, {"event": "session_gone", "tmux_session": args.tmux_session})
                break
            except Exception as exc:  # noqa: BLE001
                _log(logf, {"event": "error", "err": str(exc), "type": type(exc).__name__})

            time.sleep(_POLL_INTERVAL)
    finally:
        pidf.unlink(missing_ok=True)
        _log(logf, {"event": "stop"})


if __name__ == "__main__":  # pragma: no cover
    main()
