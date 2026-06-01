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
     Actions via inject/wait_for_handoff.
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
from .handoff import HandoffTimeoutError, validate_handoff, wait_for_handoff
from .injector import SessionGone, inject, session_exists, wait_for_cancel, wait_for_prompt
from . import stats as _stats

_POLL_INTERVAL = 15  # seconds
_KILL_SWITCH = Path.home() / ".autospec" / "no-auto-rollover.flag"
_MONITORS_DIR = Path.home() / ".autospec" / "monitors"

# Hook-mode constants
_HOOK_EVENTS = ("PreCompact", "SessionStart")


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
    - ``Action('handoff')`` injects ``adapter.command('handoff')`` then blocks
      on :func:`wait_for_handoff` for up to 180s. On
      :class:`HandoffTimeoutError`, the rollover is aborted (returns True so
      the main loop reverts engine state to COMPACTED).
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
        clear cancel window, handoff failed validation, or
        wait_for_handoff timed out); ``False`` otherwise.
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
        cmd = adapter.command("handoff") if adapter is not None else "/create-handoff"
        since = time.time()
        _log(logf, {"event": "inject", "kind": "handoff", "cmd": cmd})
        if not _prompt_ok("handoff"):
            return False
        inject(tmux_session, cmd)
        # Block until the harness writes a fresh handoff file under
        # <cwd>/.turbo/handoff/.  Must complete before /clear (next action
        # in [handoff, clear, resume]) is injected, otherwise the handoff
        # payload is lost.
        try:
            path = wait_for_handoff(
                Path(cwd) if cwd else Path.cwd(),
                since=since,
                timeout=180.0,
            )
            _log(logf, {"event": "handoff_written", "path": str(path)})
        except HandoffTimeoutError as exc:
            _log(logf, {"event": "handoff_timeout", "err": str(exc)})
            _stats.record(
                "handoff_timeout",
                harness=harness,
                tmux_session=tmux_session,
                pct=round(pct, 4),
                cwd=cwd,
            )
            return True
        return False

    if action.kind == "clear":
        # --- Handoff validation gate ---
        hfiles = (
            sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime)
            if handoff_dir.exists()
            else []
        )
        if hfiles:
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

        # --- Cancel window ---
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


# ---------------------------------------------------------------------------
# Hook-mode entry point
# ---------------------------------------------------------------------------

def _run_hook_mode(args: argparse.Namespace) -> None:
    """Handle Claude Code PreCompact / SessionStart hook invocations.

    The hook fires synchronously inside Claude Code with no tmux pane to
    inject into.  At ``PreCompact``, we read the current transcript, classify
    usage, and surface a desktop notification (via the ``claude_hook``
    adapter) so the user knows the handoff file is ready.  ``SessionStart``
    is currently a no-op (reserved for resume hints in a future revision).

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

    # PreCompact: best-effort notification path.
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
        # Resolve the latest handoff under <cwd>/.turbo/handoff and surface
        # it via the desktop notification.
        handoff_dir = Path(cwd) / ".turbo" / "handoff"
        files = (
            sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime)
            if handoff_dir.exists()
            else []
        )
        latest = files[-1] if files else None
        ClaudeHookAdapter.notify_clear_needed(handoff_path=latest)
        _log(
            logf,
            {"event": "hook_notify", "handoff": str(latest) if latest else None},
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
    adapter = _load_adapter(args.harness)
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
    try:
        while True:
            if _KILL_SWITCH.exists():
                _log(logf, {"event": "kill_switch", "reason": "no-auto-rollover.flag present"})
                break

            if not session_exists(args.tmux_session):
                _log(logf, {"event": "session_gone", "tmux_session": args.tmux_session})
                break

            try:
                transcript = adapter.find_transcript(hint)
                usage: Usage = adapter.read_usage(transcript)
                _log(logf, {
                    "event": "usage",
                    "used": usage.used_tokens,
                    "max": usage.max_tokens,
                    "pct": round(usage.used_tokens / usage.max_tokens, 4),
                    "model": usage.model,
                })
                actions = engine.classify(usage)
                for action in actions:
                    canceled = _dispatch(
                        action,
                        args.tmux_session,
                        logf,
                        harness=args.harness,
                        cwd=args.cwd,
                        pct=round(usage.used_tokens / usage.max_tokens, 4),
                        adapter=adapter,
                    )
                    if canceled:
                        # User pressed Esc during cancel window, handoff
                        # validation failed, or wait_for_handoff timed out —
                        # revert state so the engine re-fires the rollover on
                        # the next 80% climb rather than waiting for a new
                        # transcript.
                        engine._state = State.COMPACTED  # noqa: SLF001
                        _log(logf, {"event": "state_reverted", "state": "COMPACTED"})
                        break
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
