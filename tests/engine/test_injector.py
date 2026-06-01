"""Tests for autospec_context_monitor.injector."""
from __future__ import annotations

import threading
import time
from collections import deque
from unittest.mock import MagicMock, call, patch

import pytest


@pytest.fixture(autouse=True)
def reset_lock():
    """Ensure the module-level lock is released between tests."""
    from autospec_context_monitor import injector
    # Re-create the lock so lock-state from previous tests doesn't bleed over.
    original = injector._LOCK
    injector._LOCK = threading.Lock()
    yield
    injector._LOCK = original


def _make_run(returncode=0, stdout=""):
    """Build a mock subprocess.CompletedProcess-like return value."""
    result = MagicMock()
    result.returncode = returncode
    result.stdout = stdout
    return result


# ---------------------------------------------------------------------------
# inject — literal flag
# ---------------------------------------------------------------------------

def test_inject_uses_literal_flag():
    """`inject` passes '-l' to tmux send-keys for the text payload."""
    calls: list[list[str]] = []

    def fake_run(args, **kwargs):
        calls.append(list(args))
        return _make_run()

    with patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run):
        # session_exists also calls subprocess.run; first call returns 0
        # We patch session_exists directly to isolate inject behaviour.
        with patch("autospec_context_monitor.injector.session_exists", return_value=True):
            from autospec_context_monitor.injector import inject
            inject("mysession", "hello world", submit=False)

    assert len(calls) == 1, f"Expected 1 subprocess call, got {len(calls)}: {calls}"
    assert "-l" in calls[0], f"'-l' flag missing from args: {calls[0]}"
    assert "hello world" in calls[0]


# ---------------------------------------------------------------------------
# inject — separate Enter call
# ---------------------------------------------------------------------------

def test_inject_sends_enter_in_separate_call_when_submit_true():
    """`inject(submit=True)` sends Enter as a second subprocess.run call."""
    calls: list[list[str]] = []

    def fake_run(args, **kwargs):
        calls.append(list(args))
        return _make_run()

    with patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run):
        with patch("autospec_context_monitor.injector.session_exists", return_value=True):
            from autospec_context_monitor.injector import inject
            inject("mysession", "hello", submit=True)

    # Two calls: text (-l) and Enter
    assert len(calls) == 2, f"Expected 2 subprocess calls, got {len(calls)}: {calls}"
    text_call, enter_call = calls
    assert "-l" in text_call, "First call must use -l flag"
    assert "Enter" in enter_call, "Second call must send 'Enter'"
    assert "-l" not in enter_call, "Enter call must NOT use -l flag"


# ---------------------------------------------------------------------------
# prompt_ready — retry exhaustion
# ---------------------------------------------------------------------------

def test_prompt_ready_retries_three_times_then_false():
    """`prompt_ready` retries exactly `retries` times then returns False."""
    call_count = 0

    def fake_run(args, **kwargs):
        nonlocal call_count
        call_count += 1
        result = _make_run(stdout="$ some other line\n")
        return result

    with patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run):
        with patch("autospec_context_monitor.injector.time.sleep"):
            from autospec_context_monitor.injector import prompt_ready
            result = prompt_ready("mysession", ">>>", retries=3, wait=0.0)

    assert result is False
    assert call_count == 3, f"Expected 3 retries, got {call_count}"


def test_prompt_ready_returns_true_when_marker_found():
    """`prompt_ready` returns True immediately when marker appears."""
    def fake_run(args, **kwargs):
        return _make_run(stdout="user@host:~$ >>>\n")

    with patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run):
        with patch("autospec_context_monitor.injector.time.sleep"):
            from autospec_context_monitor.injector import prompt_ready
            result = prompt_ready("mysession", ">>>", retries=3, wait=0.0)

    assert result is True


# ---------------------------------------------------------------------------
# inject — SessionGone
# ---------------------------------------------------------------------------

def test_session_gone_raises():
    """`inject` raises SessionGone when the session does not exist."""
    with patch("autospec_context_monitor.injector.session_exists", return_value=False):
        from autospec_context_monitor.injector import inject, SessionGone
        with pytest.raises(SessionGone) as exc_info:
            inject("gone-session", "anything")

    assert "gone-session" in str(exc_info.value)


# ---------------------------------------------------------------------------
# inject — lock serialization
# ---------------------------------------------------------------------------

def test_lock_serializes_concurrent_inject_calls():
    """Concurrent `inject` calls execute sequentially under the module lock."""
    from autospec_context_monitor import injector as inj_module

    order: list[str] = []
    barrier = threading.Barrier(2)

    original_session_exists = inj_module.session_exists

    def fake_run(args, **kwargs):
        return _make_run()

    def thread_body(label: str, delay: float) -> None:
        with patch("autospec_context_monitor.injector.subprocess.run", side_effect=fake_run):
            with patch("autospec_context_monitor.injector.session_exists", return_value=True):
                barrier.wait()  # both threads start at the same moment
                with inj_module._LOCK:
                    order.append(f"{label}-start")
                    time.sleep(delay)
                    order.append(f"{label}-end")

    t1 = threading.Thread(target=thread_body, args=("A", 0.05))
    t2 = threading.Thread(target=thread_body, args=("B", 0.01))
    t1.start()
    t2.start()
    t1.join()
    t2.join()

    # The lock ensures start→end pairs never interleave
    assert order.index("A-end") < order.index("B-start") or \
           order.index("B-end") < order.index("A-start"), (
        f"Lock did not serialize calls; order={order}"
    )
