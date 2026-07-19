"""Handoff-file polling and validation for autospec-context-monitor.

Provides:
- :func:`wait_for_handoff` — block until a new handoff file appears.
- :func:`validate_handoff` — check that a handoff file has required headings
  and minimum size before allowing ``/clear`` to proceed.

The caller is responsible for passing the ``since`` timestamp (e.g.
``time.time()`` captured just before issuing the ``/create-handoff`` command)
so that pre-existing files are correctly ignored, modulo a tiny filesystem
timestamp-granularity slop.
"""
from __future__ import annotations

import re
import time
from datetime import date
from pathlib import Path

# Minimum byte-size for a handoff to be considered non-trivial.
_MIN_SIZE = 200
# Filesystems and wall-clock reads can disagree by a few milliseconds in fast
# same-process handoff flows. Keep stale-file filtering strict beyond that
# timestamp granularity window.
_MTIME_SLOP_SECONDS = 0.01

# Required headings (case-insensitive multiline match).
_REQUIRED_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("# heading", re.compile(r"^# \S", re.MULTILINE)),
    ("## Status", re.compile(r"^## Status\b", re.MULTILINE | re.IGNORECASE)),
    ("## Next step", re.compile(r"^## Next step\b", re.MULTILINE | re.IGNORECASE)),
]


def validate_handoff(path: Path) -> tuple[bool, list[str]]:
    """Check that *path* is a structurally valid handoff file.

    Checks (in order):
    1. File is at least :data:`_MIN_SIZE` bytes.
    2. Contains a top-level ``# <title>`` heading.
    3. Contains a ``## Status`` section.
    4. Contains a ``## Next step`` section.

    Args:
        path: Path to the handoff Markdown file.

    Returns:
        A ``(ok, missing)`` tuple where *ok* is ``True`` iff all checks pass
        and *missing* is a list of the failed check names (empty on success).
    """
    missing: list[str] = []

    try:
        size = path.stat().st_size
    except OSError:
        return False, ["size", "# heading", "## Status", "## Next step"]

    if size < _MIN_SIZE:
        missing.append("size")

    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        missing.extend(name for name, _ in _REQUIRED_PATTERNS)
        return False, missing

    for name, pattern in _REQUIRED_PATTERNS:
        if not pattern.search(text):
            missing.append(name)

    return (len(missing) == 0), missing


def _is_fresh_handoff(path: Path, since: float) -> bool:
    return path.stat().st_mtime >= since - _MTIME_SLOP_SECONDS


class HandoffTimeoutError(TimeoutError):
    """Raised when no handoff file appears within the allowed timeout."""


def check_handoff(repo_root: Path, since: float) -> Path | None:
    """Single-shot, non-blocking check for a fresh today-dated handoff.

    Unlike :func:`wait_for_handoff`, this performs exactly one filesystem scan
    and returns immediately. It lets a caller poll for the handoff across its
    own loop ticks (so the poll cadence is never frozen by a long blocking
    wait) while reusing the same match semantics.

    Args:
        repo_root: Root directory of the repository (contains ``.turbo/``).
        since:     POSIX timestamp; only files newer than the cutoff, allowing
                   a tiny filesystem timestamp slop, match.

    Returns:
        The newest matching handoff :class:`~pathlib.Path`, or ``None`` when no
        qualifying file exists yet.
    """
    handoff_dir = repo_root / ".turbo" / "handoff"
    if not handoff_dir.exists():
        return None
    today_glob = f"{date.today().isoformat()}-*.md"
    candidates = [p for p in handoff_dir.glob(today_glob) if _is_fresh_handoff(p, since)]
    if not candidates:
        return None
    return max(candidates, key=lambda p: p.stat().st_mtime)


def wait_for_handoff(
    repo_root: Path,
    since: float,
    timeout: float = 180.0,
    poll: float = 1.0,
) -> Path:
    """Block until a today-dated handoff file newer than *since* appears.

    Polls ``<repo_root>/.turbo/handoff/<YYYY-MM-DD>-*.md`` every *poll*
    seconds.  Returns the most-recently-modified matching file as soon as one
    is found.

    Args:
        repo_root: Root directory of the repository (contains ``.turbo/``).
        since:     POSIX timestamp (e.g. from :func:`time.time`). Only files
                   newer than the cutoff, allowing a tiny filesystem timestamp
                   slop, are considered.
        timeout:   Maximum number of seconds to wait (default 180).
        poll:      Polling interval in seconds (default 1.0).

    Returns:
        :class:`~pathlib.Path` to the newest matching handoff file.

    Raises:
        :class:`HandoffTimeoutError`: If no qualifying file appears before the
            deadline.  The error message includes the expected directory and
            the ``since`` cutoff so users know where to look.
    """
    deadline = time.monotonic() + timeout
    handoff_dir = repo_root / ".turbo" / "handoff"
    today_glob = f"{date.today().isoformat()}-*.md"

    while time.monotonic() < deadline:
        if handoff_dir.exists():
            candidates = [
                p
                for p in handoff_dir.glob(today_glob)
                if _is_fresh_handoff(p, since)
            ]
            if candidates:
                return max(candidates, key=lambda p: p.stat().st_mtime)
        time.sleep(poll)

    raise HandoffTimeoutError(
        f"No handoff file found in {handoff_dir} matching {today_glob!r} "
        f"with mtime > {since:.3f} within {timeout}s. "
        f"Ensure the harness wrote a file there via /create-handoff."
    )
