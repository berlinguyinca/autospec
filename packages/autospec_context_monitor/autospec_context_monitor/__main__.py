"""python -m autospec_context_monitor — context-window monitor daemon.

Usage::

    python -m autospec_context_monitor \\
        --tmux-session my-cc \\
        --harness claude \\
        --cwd /path/to/project \\
        --started-at 1717200000

The daemon:
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
import sys
import time
from pathlib import Path

from .adapters.base import TranscriptNotFoundError, Usage
from .engine import Action, Engine, State
from .injector import SessionGone, inject, session_exists, wait_for_cancel

_POLL_INTERVAL = 15  # seconds
_KILL_SWITCH = Path.home() / ".autospec" / "no-auto-rollover.flag"
_MONITORS_DIR = Path.home() / ".autospec" / "monitors"


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
    raise ValueError(f"Unknown harness: {harness!r}")


# ---------------------------------------------------------------------------
# Action dispatcher
# ---------------------------------------------------------------------------

def _dispatch(action: Action, tmux_session: str, logf: Path) -> bool:
    """Execute *action* by injecting into the tmux session.

    Returns:
        ``True`` if the action was canceled by the user (only possible for
        ``"clear"`` actions where the cancel-window fires), ``False`` otherwise.
    """
    if action.kind == "noop":
        _log(logf, {"event": "noop", "payload": action.payload})
        return False

    if action.kind == "compact":
        _log(logf, {"event": "inject", "kind": "compact"})
        inject(tmux_session, "/compact")
        return False

    if action.kind == "handoff":
        _log(logf, {"event": "inject", "kind": "handoff"})
        inject(tmux_session, "/create-handoff")
        # wait_for_handoff is handled inline during ROLLED transition in the loop
        return False

    if action.kind == "clear":
        _log(logf, {"event": "cancel_window_start", "kind": "clear"})
        if wait_for_cancel(tmux_session):
            _log(logf, {"event": "rollover canceled by user", "kind": "clear"})
            return True
        _log(logf, {"event": "inject", "kind": "clear"})
        inject(tmux_session, "/clear")
        return False

    if action.kind == "resume":
        # Find the latest handoff file and inject a resume prompt.
        handoff_dir = Path.home() / ".turbo" / "handoff"
        files = sorted(handoff_dir.glob("*.md"), key=lambda p: p.stat().st_mtime) if handoff_dir.exists() else []
        if files:
            latest = files[-1]
            resume_text = (
                f"Read {latest} and continue from where the previous session left off."
            )
        else:
            resume_text = "Continue from where the previous session left off."
        _log(logf, {"event": "inject", "kind": "resume", "file": str(files[-1]) if files else None})
        inject(tmux_session, resume_text)
        return False

    _log(logf, {"event": "unknown_action", "kind": action.kind})
    return False


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> None:
    """Entry point for the context-window monitor daemon."""
    parser = argparse.ArgumentParser(
        prog="python -m autospec_context_monitor",
        description="Context-window monitor daemon for the autospec workflow suite.",
    )
    parser.add_argument("--tmux-session", required=True, help="Tmux session name to monitor.")
    parser.add_argument(
        "--harness",
        required=True,
        choices=["claude", "codex", "opencode"],
        help="AI harness running inside the tmux session.",
    )
    parser.add_argument("--cwd", required=True, help="Working directory of the harness process.")
    parser.add_argument(
        "--started-at",
        type=float,
        default=None,
        help="Unix timestamp when the harness session started (for transcript discovery).",
    )
    args = parser.parse_args(argv)

    # ---- setup ----
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
                    canceled = _dispatch(action, args.tmux_session, logf)
                    if canceled:
                        # User pressed Esc during cancel window — revert state
                        # so the engine re-fires the rollover on the next 80%
                        # climb rather than waiting for a new transcript.
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
