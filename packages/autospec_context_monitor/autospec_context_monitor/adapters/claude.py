"""Claude Code adapter for autospec-context-monitor.

Implements the HarnessAdapter Protocol for Claude Code, locating transcripts
under ``~/.claude/projects/<cwd-slug>/`` and summing token usage from JSONL.
"""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Literal

from .base import HarnessAdapter, TranscriptNotFoundError, Usage

# Transcript directories checked by `doctor.py` for write-access.
PATHS_TO_CHECK: list[Path] = [
    Path.home() / ".claude" / "projects",
]

# Context-window sizes by model identifier.
# Add new models here as Claude Code ships them.
MODEL_MAX: dict[str, int] = {
    "claude-sonnet-4-5": 200_000,
    "claude-sonnet-4-5-1m": 1_000_000,
    "claude-sonnet-4-6": 200_000,
    "claude-sonnet-4-7": 200_000,
    "claude-opus-4-7": 200_000,
    "claude-opus-4-5": 200_000,
    "claude-opus-4-8": 200_000,
    "claude-opus-4-8-1m": 1_000_000,
    "claude-opus-4-8[1m]": 1_000_000,
    "claude-haiku-4-5": 200_000,
    # Legacy identifiers
    "claude-3-5-sonnet-20241022": 200_000,
    "claude-3-5-haiku-20241022": 200_000,
    "claude-3-opus-20240229": 200_000,
}

_DEFAULT_MAX = 200_000


def _resolve_max(model: str | None) -> int:
    """Context-window size for a model id.

    Exact ``MODEL_MAX`` entries win; otherwise any id carrying the 1M-context
    marker (a ``-1m`` suffix or a ``[1m]`` tag, the convention Claude Code uses
    for million-token sessions) resolves to 1_000_000 so a newly-shipped 1M
    model is not silently capped at the 200k default (issue #898). Everything
    else falls back to ``_DEFAULT_MAX``.
    """
    if not model:
        return _DEFAULT_MAX
    if model in MODEL_MAX:
        return MODEL_MAX[model]
    if model.endswith("-1m") or "[1m]" in model:
        return 1_000_000
    return _DEFAULT_MAX

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

        Derives the project slug from ``hint["cwd"]`` the way Claude Code does:
        every non-alphanumeric character (``/``, ``.``, ``_``, space, …) is
        replaced with ``-``. The cwd is absolute, so it starts with ``/`` and
        the slug keeps a leading ``-`` (e.g. ``/Users/x/my_repo`` →
        ``-Users-x-my-repo``) — do NOT strip it, or the path will not match the
        directory Claude Code actually creates. Verified against live
        ``~/.claude/projects`` slugs, where underscores in the source path
        (e.g. ``.../m_/...``) appear as ``-``.

        Raises :class:`~autospec_context_monitor.adapters.base.TranscriptNotFoundError`
        if no transcript exists.
        """
        cwd: str = hint.get("cwd", "")
        slug = re.sub(r"[^a-zA-Z0-9]", "-", cwd)
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
        max_tokens = _resolve_max(model)
        return Usage(
            used_tokens=used,
            max_tokens=max_tokens,
            model=model,
            estimated=False,
        )

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command to the Claude Code slash command or prompt."""
        return _COMMAND_MAP[logical]
