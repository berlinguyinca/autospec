"""Handoff-file polling for autospec-context-monitor.

Provides :func:`wait_for_handoff` which blocks until a new handoff Markdown
file appears in ``<repo_root>/.turbo/handoff/`` with a modification time
strictly after ``since``.

The caller is responsible for passing the ``since`` timestamp (e.g.
``time.time()`` captured just before issuing the ``/create-handoff`` command)
so that pre-existing files are correctly ignored.
"""
from __future__ import annotations

import time
from datetime import date
from pathlib import Path


class HandoffTimeoutError(TimeoutError):
    """Raised when no handoff file appears within the allowed timeout."""


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
        since:     POSIX timestamp (e.g. from :func:`time.time`).  Only files
                   with ``st_mtime > since`` are considered.
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
                if p.stat().st_mtime > since
            ]
            if candidates:
                return max(candidates, key=lambda p: p.stat().st_mtime)
        time.sleep(poll)

    raise HandoffTimeoutError(
        f"No handoff file found in {handoff_dir} matching {today_glob!r} "
        f"with mtime > {since:.3f} within {timeout}s. "
        f"Ensure the harness wrote a file there via /create-handoff."
    )
