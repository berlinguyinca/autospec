#!/usr/bin/env python3
"""Rolling-window queue arithmetic for the dual-Turing node.

llama.cpp publishes no completed-request counter, so per-request service time
cannot be read. This module infers it from the one thing that is observable: the
number of outstanding requests going DOWN.

Every accessor returns None rather than a plausible number when it has not
observed enough to know. "We do not know yet" is a valid answer; a fabricated
ETA is not, and a confident wrong number is worse than a blank.

No I/O and no clock reads -- the caller supplies timestamps, which is what makes
the eviction and rate arithmetic testable without sleeping.
"""
from __future__ import annotations

from collections import deque


class QueueWindow:
    def __init__(self, window_seconds: float = 300.0) -> None:
        self.window_seconds = float(window_seconds)
        # (timestamp, outstanding, completions_since_prev, busy_seconds_since_prev)
        self._samples: deque[tuple[float, int, int, float]] = deque()

    # --- ingest ---------------------------------------------------------
    def add(self, ts: float, outstanding: int) -> None:
        ts = float(ts)
        outstanding = max(0, int(outstanding))
        if self._samples and ts <= self._samples[-1][0]:
            # Out-of-order or duplicate timestamp: a negative interval would
            # corrupt busy_seconds, so drop it rather than "fix" it.
            return
        completions = 0
        busy = 0.0
        if self._samples:
            prev_ts, prev_out, _, _ = self._samples[-1]
            if outstanding < prev_out:
                completions = prev_out - outstanding
            if prev_out > 0:
                # The interval counts as busy because work was outstanding at its
                # start. Wall-clock time spent idle must not dilute the rate.
                busy = ts - prev_ts
        self._samples.append((ts, outstanding, completions, busy))
        self._evict(ts)

    def _evict(self, now: float) -> None:
        cutoff = now - self.window_seconds
        while self._samples and self._samples[0][0] < cutoff:
            self._samples.popleft()

    # --- observations ---------------------------------------------------
    @property
    def samples(self) -> int:
        return len(self._samples)

    @property
    def completions(self) -> int:
        return sum(s[2] for s in self._samples)

    @property
    def busy_seconds(self) -> float:
        return sum(s[3] for s in self._samples)

    # --- derived --------------------------------------------------------
    def service_rate(self) -> float | None:
        """Completions per BUSY second, or None if nothing has completed."""
        c = self.completions
        b = self.busy_seconds
        if c <= 0 or b <= 0:
            return None
        return c / b

    def mean_service_seconds(self) -> float | None:
        r = self.service_rate()
        return None if r is None else 1.0 / r

    def est_wait_seconds(self, requests_ahead: int) -> float | None:
        """How long until `requests_ahead` requests have cleared.

        None when the rate is unknown. Zero when nothing is ahead -- that is a
        fact, not an estimate.
        """
        if requests_ahead <= 0:
            return 0.0
        r = self.service_rate()
        return None if r is None else requests_ahead / r
