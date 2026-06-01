"""tmux injection layer for autospec-context-monitor.

Provides safe, guarded injection of text into a tmux pane with:
- ``session_exists`` — check tmux session liveness.
- ``prompt_ready`` — poll pane output for a prompt marker before injecting.
- ``inject`` — send text via ``tmux send-keys -l`` (literal mode) then a
  separate ``Enter`` keystroke to prevent shell-meta chaining.
- ``notify_failure`` — ring terminal bell and send a system notification when
  guards prevent injection.

A module-level :class:`threading.Lock` serialises concurrent ``inject`` calls
within the same process.
"""
from __future__ import annotations

import subprocess
import sys
import threading
import time
from pathlib import Path


class SessionGone(RuntimeError):
    """Raised when the target tmux session no longer exists."""

    def __init__(self, session: str) -> None:
        super().__init__(f"tmux session {session!r} does not exist")
        self.session = session


# Per-process lock: only one inject call runs at a time.
_LOCK = threading.Lock()


def session_exists(session: str) -> bool:
    """Return ``True`` if *session* exists in tmux, ``False`` otherwise."""
    result = subprocess.run(
        ["tmux", "has-session", "-t", session],
        capture_output=True,
    )
    return result.returncode == 0


def prompt_ready(session: str, marker: str, retries: int = 3, wait: float = 2.0) -> bool:
    """Poll the pane bottom line until it ends with *marker*.

    Tries up to *retries* times with *wait* seconds between attempts.
    Returns ``True`` on success, ``False`` if the marker never appears.
    """
    for _ in range(retries):
        result = subprocess.run(
            ["tmux", "capture-pane", "-p", "-t", session],
            capture_output=True,
            text=True,
            timeout=5,
        )
        lines = result.stdout.splitlines()
        if lines and lines[-1].strip().endswith(marker):
            return True
        time.sleep(wait)
    return False


def inject(session: str, text: str, *, submit: bool = True) -> None:
    """Inject *text* into *session* using literal-mode ``send-keys``.

    Sends the text with ``tmux send-keys -l -t <session> <text>`` to prevent
    key-name interpretation, then (if *submit* is ``True``) sends ``Enter``
    as a separate call to prevent shell-meta chaining.

    Raises :class:`SessionGone` if the session no longer exists.
    Acquires the module-level lock to prevent concurrent injections.
    """
    with _LOCK:
        if not session_exists(session):
            raise SessionGone(session)
        subprocess.run(
            ["tmux", "send-keys", "-l", "-t", session, text],
            check=True,
            timeout=5,
        )
        if submit:
            subprocess.run(
                ["tmux", "send-keys", "-t", session, "Enter"],
                check=True,
                timeout=5,
            )


def wait_for_prompt(session: str, marker: str, attempts: int = 3, delay: float = 2.0) -> bool:
    """Poll the pane output until *marker* appears; return ``False`` after exhausting attempts.

    This is the public guard function used by ``_dispatch`` before each inject.
    It delegates to :func:`prompt_ready` with the spec-aligned parameter names
    (``attempts`` / ``delay`` instead of ``retries`` / ``wait``).

    Args:
        session:  Tmux session name.
        marker:   String to look for in the captured pane output.
        attempts: Maximum number of polling attempts (default 3).
        delay:    Seconds to wait between attempts (default 2.0).

    Returns:
        ``True`` when the marker is seen; ``False`` after all attempts are exhausted.
    """
    return prompt_ready(session, marker, retries=attempts, wait=delay)


def wait_for_cancel(tmux_session: str, timeout: int = 10) -> bool:
    """Show a countdown overlay and return ``True`` if the user pressed Esc.

    Binds ``Escape`` in the tmux root key-table to ``touch``-create a sentinel
    file at ``~/.autospec/monitors/<tmux_session>.cancel``.  Polls every second
    for up to *timeout* seconds, refreshing a ``tmux display-message`` countdown
    each tick.  Always unbinds the key and removes the sentinel in a ``finally``
    block, regardless of how the function exits.

    Args:
        tmux_session: Name of the tmux session to target.
        timeout:      Number of seconds to count down (default 10).

    Returns:
        ``True`` if the sentinel file was created (user pressed Esc),
        ``False`` if the countdown expired without cancellation.
    """
    monitors_dir = Path.home() / ".autospec" / "monitors"
    monitors_dir.mkdir(parents=True, exist_ok=True)
    sentinel = monitors_dir / f"{tmux_session}.cancel"
    sentinel.unlink(missing_ok=True)

    # Bind Escape to touch the sentinel file.
    subprocess.run(
        [
            "tmux", "bind-key", "-T", "root", "Escape",
            "run-shell", f"touch {sentinel}",
        ],
        check=False,
    )

    canceled = False
    try:
        for remaining in range(timeout, 0, -1):
            subprocess.run(
                [
                    "tmux", "display-message", "-d", "1000",
                    f"autospec: rolling in {remaining}s — Esc to cancel",
                ],
                check=False,
            )
            time.sleep(1)
            if sentinel.exists():
                canceled = True
                break
    finally:
        subprocess.run(
            ["tmux", "unbind-key", "-T", "root", "Escape"],
            check=False,
        )
        sentinel.unlink(missing_ok=True)

    return canceled


def notify_failure(message: str) -> None:
    """Notify the user that an injection guard prevented a command.

    Fires a terminal bell and a system desktop notification via
    ``osascript`` (macOS) or ``notify-send`` (Linux).  Both calls are
    best-effort (``check=False``) so a missing notifier never aborts the
    monitor loop.
    """
    # Terminal bell — works in any terminal emulator.
    sys.stdout.write("\a")
    sys.stdout.flush()

    # tmux status-bar message (best-effort; tmux may not be attached).
    subprocess.run(["tmux", "display-message", message], check=False)

    if sys.platform == "darwin":
        subprocess.run(
            ["osascript", "-e", f'display notification "{message}" with title "autospec"'],
            check=False,
        )
    else:
        subprocess.run(["notify-send", "autospec", message], check=False)
