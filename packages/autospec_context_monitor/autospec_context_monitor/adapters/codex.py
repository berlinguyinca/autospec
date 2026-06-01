"""Codex CLI adapter for autospec-context-monitor.

Implements the HarnessAdapter Protocol for Codex CLI, locating rollout
transcripts under ``~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`` and
summing token usage from ``event_msg.payload.info``.  When ``info`` is
``null`` or absent, falls back to character-count estimation and sets
``estimated=True``.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Literal

from .base import HarnessAdapter, TranscriptNotFoundError, Usage

# Context-window sizes by model identifier.
MODEL_MAX: dict[str, int] = {
    "gpt-5": 200_000,
    "gpt-5-codex": 200_000,
    "gpt-4o": 128_000,
    "gpt-4o-mini": 128_000,
    "o1": 200_000,
    "o1-mini": 128_000,
    "o3": 200_000,
    "o3-mini": 200_000,
}

_DEFAULT_MAX = 128_000

_HANDOFF_PROMPT = (
    "Write a handoff file to .turbo/handoff/<YYYY-MM-DD>-<slug>.md capturing "
    "current task, status, open decisions, in-flight changes, and next step. "
    "Confirm the path when done."
)

_COMMAND_MAP: dict[str, str] = {
    "clear": "/new",
    "compact": "/compact",
    "handoff": _HANDOFF_PROMPT,
}

# Chars-per-token heuristic used when explicit counts are unavailable.
_CHARS_PER_TOKEN = 3.5


class CodexAdapter:
    """HarnessAdapter implementation for Codex CLI."""

    name: str = "codex"

    def find_transcript(self, hint: dict) -> Path:
        """Locate the newest ``rollout-*.jsonl`` transcript.

        Searches ``~/.codex/sessions/**/rollout-*.jsonl`` recursively and
        returns the most-recently-modified match.

        Raises :class:`~autospec_context_monitor.adapters.base.TranscriptNotFoundError`
        if no transcript is found.
        """
        root = Path.home() / ".codex" / "sessions"
        candidates = sorted(
            root.rglob("rollout-*.jsonl"),
            key=lambda p: p.stat().st_mtime,
            reverse=True,
        )
        if not candidates:
            raise TranscriptNotFoundError(
                f"No Codex rollout transcript found under {root}"
            )
        return candidates[0]

    def read_usage(self, transcript: Path) -> Usage:
        """Sum token usage from ``event_msg.payload.info``.

        When ``info`` is ``null`` or absent for every line, estimates token
        count from character length (``len(raw) / 3.5``) and sets
        ``estimated=True``.  Partial files with a mix of explicit and null
        entries use explicit counts only and leave ``estimated=False``.
        """
        used = 0
        estimated = False
        model = "unknown"
        text_total = 0
        has_explicit = False

        for raw in transcript.read_text(encoding="utf-8").splitlines():
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue

            # Extract model
            m = obj.get("model")
            if not m:
                m = (
                    obj.get("event_msg", {})
                    .get("payload", {})
                    .get("model")
                )
            if m:
                model = m

            # Extract token counts
            info = obj.get("event_msg", {}).get("payload", {}).get("info")
            if info:
                used += info.get("input_tokens", 0) + info.get("output_tokens", 0)
                has_explicit = True
            else:
                # Accumulate raw bytes for fallback estimation
                text_total += len(raw)

        if not has_explicit and text_total > 0:
            used = int(text_total / _CHARS_PER_TOKEN)
            estimated = True

        return Usage(
            used_tokens=used,
            max_tokens=MODEL_MAX.get(model, _DEFAULT_MAX),
            model=model,
            estimated=estimated,
        )

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command to the Codex CLI command or prompt."""
        return _COMMAND_MAP[logical]
