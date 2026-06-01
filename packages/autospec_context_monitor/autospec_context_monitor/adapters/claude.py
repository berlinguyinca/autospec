"""Claude Code adapter for autospec-context-monitor.

Implements the HarnessAdapter Protocol for Claude Code, locating transcripts
under ``~/.claude/projects/<cwd-slug>/`` and summing token usage from JSONL.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Literal

from .base import HarnessAdapter, TranscriptNotFoundError, Usage

# Context-window sizes by model identifier.
# Add new models here as Claude Code ships them.
MODEL_MAX: dict[str, int] = {
    "claude-sonnet-4-5": 200_000,
    "claude-sonnet-4-5-1m": 1_000_000,
    "claude-sonnet-4-6": 200_000,
    "claude-sonnet-4-7": 200_000,
    "claude-opus-4-7": 200_000,
    "claude-opus-4-5": 200_000,
    "claude-haiku-4-5": 200_000,
    # Legacy identifiers
    "claude-3-5-sonnet-20241022": 200_000,
    "claude-3-5-haiku-20241022": 200_000,
    "claude-3-opus-20240229": 200_000,
}

_DEFAULT_MAX = 200_000

_HANDOFF_PROMPT = (
    "Please run /create-handoff and wait for the file to be written "
    "before responding further."
)

_COMMAND_MAP: dict[str, str] = {
    "clear": "/clear",
    "compact": "/compact",
    "handoff": _HANDOFF_PROMPT,
}


class ClaudeAdapter:
    """HarnessAdapter implementation for Claude Code (claude CLI)."""

    name: str = "claude"

    def find_transcript(self, hint: dict) -> Path:
        """Locate the newest ``.jsonl`` transcript for this session.

        Derives the project slug from ``hint["cwd"]`` (``/``→``-``, strip
        leading ``-``), then returns the most-recently-modified ``.jsonl``
        found at ``~/.claude/projects/<slug>/``.

        Raises :class:`~autospec_context_monitor.adapters.base.TranscriptNotFoundError`
        if no transcript exists.
        """
        cwd: str = hint.get("cwd", "")
        slug = cwd.replace("/", "-").lstrip("-")
        root = Path.home() / ".claude" / "projects" / slug
        candidates = sorted(
            root.glob("*.jsonl"),
            key=lambda p: p.stat().st_mtime,
            reverse=True,
        )
        if not candidates:
            raise TranscriptNotFoundError(
                f"No Claude transcript found under {root} (cwd={cwd!r})"
            )
        return candidates[0]

    def read_usage(self, transcript: Path) -> Usage:
        """Sum ``input_tokens + output_tokens`` from every JSONL line that has
        ``message.usage``.  The last ``message.model`` seen is used for the
        model-name lookup.
        """
        used = 0
        model = "unknown"
        for raw in transcript.read_text(encoding="utf-8").splitlines():
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue
            msg = obj.get("message", {})
            u = msg.get("usage")
            if u:
                used += u.get("input_tokens", 0) + u.get("output_tokens", 0)
            m = msg.get("model")
            if m:
                model = m
        max_tokens = MODEL_MAX.get(model, _DEFAULT_MAX)
        return Usage(
            used_tokens=used,
            max_tokens=max_tokens,
            model=model,
            estimated=False,
        )

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command to the Claude Code slash command or prompt."""
        return _COMMAND_MAP[logical]
