"""OpenCode adapter for autospec-context-monitor.

Implements the HarnessAdapter Protocol for OpenCode, reading token usage from
``~/.local/share/opencode/opencode.db`` in read-only URI mode.  Falls back to
``opencode export <session_id>`` subprocess on schema mismatch (OperationalError).
"""
from __future__ import annotations

import sqlite3
import subprocess
from pathlib import Path
from typing import Literal

from .base import HarnessAdapter, TranscriptNotFoundError, Usage

# Context-window sizes by model identifier.
MODEL_MAX: dict[str, int] = {
    "claude-sonnet-4-5": 200_000,
    "claude-sonnet-4-6": 200_000,
    "claude-sonnet-4-7": 200_000,
    "claude-opus-4-5": 200_000,
    "claude-opus-4-7": 200_000,
    "gpt-4o": 128_000,
    "gpt-4-turbo": 128_000,
}

_DEFAULT_MAX = 128_000

_HANDOFF_PROMPT = (
    "Write a handoff file to .turbo/handoff/<YYYY-MM-DD>-<slug>.md capturing "
    "current task, status, open decisions, in-flight changes, and next step."
)

_COMMAND_MAP: dict[str, str] = {
    "clear": "/new",
    "compact": "/compact",
    "handoff": _HANDOFF_PROMPT,
}

_DB_PATH = Path.home() / ".local" / "share" / "opencode" / "opencode.db"


class OpenCodeAdapter:
    """HarnessAdapter implementation for OpenCode."""

    name: str = "opencode"

    # Class-level default; overridden per-instance when find_transcript is called.
    _session_id: str = ""

    def find_transcript(self, hint: dict) -> Path:
        """Return the OpenCode database path, storing session_id from hint.

        The *hint* dict may contain ``session_id`` directly, or it will be
        derived from ``tmux_session`` or left empty.

        Raises :class:`~autospec_context_monitor.adapters.base.TranscriptNotFoundError`
        if the database file does not exist.
        """
        db = _DB_PATH
        if not db.exists():
            raise TranscriptNotFoundError(
                f"OpenCode database not found at {db}. "
                "Ensure OpenCode has been run at least once."
            )
        # Cache the session_id for use in read_usage.
        self._session_id = (
            hint.get("session_id")
            or hint.get("tmux_session")
            or ""
        )
        return db

    def read_usage(self, transcript: Path) -> Usage:
        """Return a Usage snapshot for the current session.

        Opens the SQLite database in read-only URI mode to avoid locking the
        live writer.  On ``OperationalError`` (schema mismatch / renamed table),
        falls back to ``opencode export <session_id>``.
        """
        sid = self._session_id
        try:
            uri = f"file:{transcript}?mode=ro"
            con = sqlite3.connect(uri, uri=True, timeout=2)
            try:
                row = con.execute(
                    "SELECT COALESCE(SUM(input_tokens + output_tokens), 0), MAX(model) "
                    "FROM messages WHERE session_id = ?",
                    (sid,),
                ).fetchone()
            finally:
                con.close()
            used: int = row[0] if row and row[0] is not None else 0
            model: str = (row[1] if row and row[1] else "unknown")
        except sqlite3.OperationalError:
            used, model = self._export_fallback(sid)

        max_tokens = MODEL_MAX.get(model, _DEFAULT_MAX)
        return Usage(
            used_tokens=used,
            max_tokens=max_tokens,
            model=model,
            estimated=False,
        )

    def _export_fallback(self, sid: str) -> tuple[int, str]:
        """Call ``opencode export <sid>`` and parse token totals from its output.

        Each line of the export may contain ``input_tokens`` and/or
        ``output_tokens`` integer fields.  We sum them and pick the last
        non-empty ``model`` field seen.

        Raises :class:`subprocess.CalledProcessError` if opencode CLI is absent
        or exits non-zero, with a message directing the user to install OpenCode.
        """
        try:
            result = subprocess.run(
                ["opencode", "export", sid],
                capture_output=True,
                text=True,
                timeout=10,
                check=True,
            )
        except FileNotFoundError as exc:
            raise RuntimeError(
                "OpenCode database schema mismatch and 'opencode' CLI not found. "
                "Install OpenCode or check your PATH."
            ) from exc
        except subprocess.CalledProcessError as exc:
            raise RuntimeError(
                f"'opencode export {sid}' failed (exit {exc.returncode}): {exc.stderr.strip()}. "
                "Check that the session ID is valid."
            ) from exc

        import json

        used = 0
        model = "unknown"
        for line in result.stdout.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            used += obj.get("input_tokens", 0) + obj.get("output_tokens", 0)
            m = obj.get("model")
            if m:
                model = m
        return used, model

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map a logical command to the OpenCode slash command or prompt."""
        return _COMMAND_MAP[logical]
