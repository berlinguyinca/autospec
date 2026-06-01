"""Claude Code native-hook adapter for autospec-context-monitor.

Implements the HarnessAdapter Protocol for Claude Code environments where
``tmux`` is not available.  At 80% context usage, rather than injecting
``/clear`` into a tmux pane, the monitor writes the handoff file path to
stdout and fires a desktop notification asking the user to run ``/clear``
manually.  This is signalled by returning the sentinel string
``__HOOK_NOTIFY__`` from :meth:`command`.

``find_transcript`` and ``read_usage`` delegate entirely to
:class:`~autospec_context_monitor.adapters.claude.ClaudeAdapter` so token
accounting is identical between the two modes.

The sentinel is interpreted by the monitor's ``_dispatch`` function, which
routes it to ``osascript`` (macOS) or ``notify-send`` (Linux) with a
human-readable message.

Install integration:
    ``install.sh --hook-mode claude`` writes ``hooks.PreCompact`` and
    ``hooks.SessionStart`` to ``~/.claude/settings.json`` (merge, not
    overwrite) so Claude Code invokes ``python -m autospec_context_monitor
    --hook-event PreCompact`` on every compaction opportunity.
"""
from __future__ import annotations

import sys
import subprocess
from pathlib import Path
from typing import Literal

from .base import Usage
from .claude import ClaudeAdapter

# Sentinel returned by command("clear") in hook mode.  The monitor treats
# this as a request to notify the user rather than inject into a tmux pane.
HOOK_NOTIFY_SENTINEL = "__HOOK_NOTIFY__"

_DELEGATE = ClaudeAdapter()


class ClaudeHookAdapter:
    """HarnessAdapter for Claude Code without tmux (PreCompact hook mode).

    Delegates transcript discovery and usage reading to the standard
    :class:`~autospec_context_monitor.adapters.claude.ClaudeAdapter`.
    Overrides ``command("clear")`` to return a sentinel that triggers a
    desktop notification instead of a tmux ``send-keys`` injection.
    """

    name: str = "claude"

    def find_transcript(self, hint: dict) -> Path:
        """Locate the newest Claude Code ``.jsonl`` transcript.

        Delegates to :meth:`ClaudeAdapter.find_transcript`.
        """
        return _DELEGATE.find_transcript(hint)

    def read_usage(self, transcript: Path) -> Usage:
        """Sum token usage from *transcript*.

        Delegates to :meth:`ClaudeAdapter.read_usage`.
        """
        return _DELEGATE.read_usage(transcript)

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command to a harness string.

        ``"clear"`` returns :data:`HOOK_NOTIFY_SENTINEL` — the monitor routes
        this to a desktop notification rather than a tmux injection.
        All other logical commands delegate to the standard
        :class:`ClaudeAdapter`.
        """
        if logical == "clear":
            return HOOK_NOTIFY_SENTINEL
        return _DELEGATE.command(logical)

    # ------------------------------------------------------------------
    # Hook-mode notification helper (called by monitor when sentinel seen)
    # ------------------------------------------------------------------

    @staticmethod
    def notify_clear_needed(handoff_path: Path | None = None) -> None:
        """Fire a desktop notification asking the user to run ``/clear``.

        On macOS uses ``osascript``; on Linux uses ``notify-send``.
        Both calls are best-effort (``check=False``) so a missing notifier
        never aborts the monitor loop.

        Args:
            handoff_path: Path to the handoff file that was written, included
                in the notification body when provided.
        """
        body = "Context window full — please run /clear to continue."
        if handoff_path is not None:
            body = f"{body} Handoff: {handoff_path}"

        if sys.platform == "darwin":
            subprocess.run(
                [
                    "osascript",
                    "-e",
                    f'display notification "{body}" with title "autospec: rollover needed"',
                ],
                check=False,
            )
        else:
            subprocess.run(
                ["notify-send", "autospec: rollover needed", body],
                check=False,
            )
