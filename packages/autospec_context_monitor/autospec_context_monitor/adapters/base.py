"""Base adapter contract for autospec-context-monitor.

Defines the frozen Usage dataclass, the HarnessAdapter Protocol, and
TranscriptNotFoundError — the three shared types every concrete adapter
must satisfy.
"""
from __future__ import annotations

import dataclasses
from pathlib import Path
from typing import Literal, Protocol, runtime_checkable


@dataclasses.dataclass(frozen=True)
class Usage:
    """Token-usage snapshot returned by a harness adapter.

    Attributes:
        used_tokens: Tokens consumed so far in this session.
        max_tokens:  Context-window size for the active model.
        model:       Model identifier string (e.g. ``"claude-sonnet-4-6"``).
        estimated:   ``True`` when counts are derived from text length rather
                     than explicit API metadata.
    """

    used_tokens: int
    max_tokens: int
    model: str
    estimated: bool

    @property
    def pct(self) -> float:
        """Return the fraction of context consumed (``used_tokens / max_tokens``)."""
        if self.max_tokens == 0:
            return 0.0
        return self.used_tokens / self.max_tokens


class TranscriptNotFoundError(FileNotFoundError):
    """Raised by :meth:`HarnessAdapter.find_transcript` when no live transcript
    can be located within the allowed wait period."""


@runtime_checkable
class HarnessAdapter(Protocol):
    """Per-harness adapter contract.

    Concrete implementations live in sibling modules (``adapters/claude.py``,
    ``adapters/codex.py``, ``adapters/opencode.py``).  The engine only calls
    these three methods so adapters remain fully decoupled from I/O plumbing.
    """

    name: str  # "claude" | "codex" | "opencode"

    def find_transcript(self, hint: dict) -> Path:
        """Locate the live transcript for this session.

        *hint* contains ``{cwd, tmux_session, pid, started_at}``.  Returns the
        newest matching transcript path, or raises :class:`TranscriptNotFoundError`
        after a 30-second wait.
        """
        ...

    def read_usage(self, transcript: Path) -> Usage:
        """Return a :class:`Usage` snapshot from *transcript*.

        Set ``estimated=True`` when token counts are derived from character
        length rather than explicit API metadata.
        """
        ...

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command name to the harness-specific slash command."""
        ...

    def prompt_marker(self) -> str:
        """Return a string that identifies an idle shell prompt in the tmux pane.

        ``_dispatch`` captures pane output and checks for this marker before
        injecting a command.  Each adapter should return a marker that reliably
        appears at the end of the prompt line when the harness is idle and ready
        to accept input (e.g. ``"> "`` for Claude Code, ``"$ "`` for a bare
        shell, ``"% "`` for zsh).
        """
        ...
