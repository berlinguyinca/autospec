"""Rolling-window queue arithmetic.

Every accessor must return None rather than a plausible number when it has not
observed enough to know. The whole point of this module is that "we do not know
yet" is a valid answer and a fabricated ETA is not.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "queue_window.py"


def load():
    spec = importlib.util.spec_from_file_location("queue_window", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def W(window=300.0):
    return load().QueueWindow(window_seconds=window)


# --- nothing observed ------------------------------------------------------

def test_empty_window_knows_nothing():
    w = W()
    assert w.samples == 0
    assert w.completions == 0
    assert w.service_rate() is None
    assert w.mean_service_seconds() is None
    assert w.est_wait_seconds(3) is None


def test_idle_only_still_knows_nothing():
    """An idle node has not proven it is fast."""
    w = W()
    for i in range(10):
        w.add(float(i), 0)
    assert w.completions == 0
    assert w.est_wait_seconds(2) is None


# --- completions are decreases in outstanding ------------------------------

def test_counts_a_single_completion():
    w = W()
    w.add(0.0, 1)
    w.add(1.0, 0)
    assert w.completions == 1


def test_counts_multi_request_drop_as_multiple_completions():
    w = W()
    w.add(0.0, 3)
    w.add(1.0, 1)
    assert w.completions == 2


def test_increase_is_not_a_completion():
    w = W()
    w.add(0.0, 0)
    w.add(1.0, 2)
    w.add(2.0, 5)
    assert w.completions == 0


# --- busy seconds exclude idle --------------------------------------------

def test_busy_seconds_only_counts_time_with_work_outstanding():
    w = W()
    w.add(0.0, 0)    # idle
    w.add(1.0, 1)    # becomes busy at t=1
    w.add(3.0, 0)    # idle again at t=3 -> 2 busy seconds
    w.add(9.0, 0)    # still idle
    assert w.busy_seconds == 2.0


def test_service_rate_divides_by_busy_not_wall_clock():
    """A node idle 8 of 10 seconds has not become slow."""
    w = W()
    w.add(0.0, 0)
    w.add(1.0, 1)
    w.add(3.0, 0)
    w.add(9.0, 0)
    assert w.completions == 1
    assert abs(w.service_rate() - 0.5) < 1e-9
    assert abs(w.mean_service_seconds() - 2.0) < 1e-9


# --- the estimate ---------------------------------------------------------

def test_est_wait_uses_service_rate():
    w = W()
    w.add(0.0, 2)
    w.add(2.0, 0)          # 2 completions in 2 busy seconds -> 1/s
    assert abs(w.est_wait_seconds(4) - 4.0) < 1e-9


def test_est_wait_of_nothing_ahead_is_zero_not_none():
    w = W()
    w.add(0.0, 2)
    w.add(2.0, 0)
    assert w.est_wait_seconds(0) == 0.0


def test_est_wait_is_none_without_completions_even_with_samples():
    w = W()
    for i in range(50):
        w.add(float(i), 2)          # permanently busy, nothing finished
    assert w.samples == 50
    assert w.completions == 0
    assert w.est_wait_seconds(1) is None


# --- eviction ------------------------------------------------------------

def test_old_samples_leave_the_window():
    w = W(window=10.0)
    w.add(0.0, 1)
    w.add(1.0, 0)          # a completion at t=1
    assert w.completions == 1
    w.add(100.0, 0)        # far outside the window
    assert w.completions == 0
    assert w.est_wait_seconds(1) is None


def test_window_keeps_recent_samples():
    w = W(window=10.0)
    w.add(0.0, 1)
    w.add(1.0, 0)
    w.add(5.0, 0)
    assert w.completions == 1


# --- robustness ----------------------------------------------------------

def test_out_of_order_timestamps_are_ignored_not_fatal():
    w = W()
    w.add(5.0, 1)
    w.add(1.0, 0)          # earlier than the last sample
    assert w.samples == 1


def test_negative_outstanding_is_clamped():
    w = W()
    w.add(0.0, -3)
    assert w.samples == 1
    assert w.completions == 0
