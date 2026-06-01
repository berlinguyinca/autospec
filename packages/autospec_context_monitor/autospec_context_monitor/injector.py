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
