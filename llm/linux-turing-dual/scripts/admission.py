#!/usr/bin/env python3
"""Admission control for the shared KV pool: hold, rather than over-subscribe.

A context tier on this node has always been a CONTRACT and never a limit. The
pool is one allocation per loaded model, divided into slots, and nothing stopped
two sessions from claiming more of it than exists:

    qwen3.5-9b       81920 tokens, 2 slots -> 40960 each
    qwen3.5-9b-80k   the whole pool, by name

An `-80k` session beside a `-40k` one asks for 122880 of 81920, and llama.cpp does
not refuse -- it dies, taking every live session with it. Documented on this node
for months as "a shared KV pool has no admission control". This is that control.

The rule is to WAIT, not to refuse. Many small agents arriving at once is the
expected shape of this workload, and a queue turns a thundering herd into a line;
a refusal would turn it into a retry storm, which is the same herd with worse
latency and no ordering.

Three properties this must have, and each is tested:

  * FIFO. A large request waits behind nothing and ahead of everything that came
    later. Admitting whoever happens to fit starves the big session forever --
    which is the one caller most likely to be a person waiting.
  * A request that can NEVER fit is refused at once, never queued. Waiting for
    capacity that cannot exist is a hang dressed up as fairness.
  * Every admission is released on every path. A leaked lease shrinks the pool
    permanently, and the failure looks like a node that got mysteriously slower.
"""
from __future__ import annotations

import re
import threading
import time

# A tier named in an id: `-40k`, `-80k`, `-100k`. This is the caller's own
# statement of what they intend to use, which is exactly what should be reserved.
_TIER = re.compile(r"-(\d+)k(?:$|-)")


class TooLarge(Exception):
    """The request cannot fit the pool even when the pool is empty."""


class Lease:
    __slots__ = ("tokens", "waited")

    def __init__(self, tokens: int, waited: float) -> None:
        self.tokens = tokens
        self.waited = waited


def tier_tokens(model_id: str, context: int, slots: int) -> int:
    """How much of the pool this request is claiming.

    A named tier is taken at its word: `-80k` means 80 * 1024. An id with no tier
    gets the FAIR SHARE the preset was configured for -- `context / slots` -- which
    is what "two seats" already meant. Guessing the whole pool instead would
    serialise ordinary traffic; guessing a token would defeat the control.
    """
    m = _TIER.search(model_id or "")
    if m:
        return int(m.group(1)) * 1024
    if context and slots:
        return max(1, int(context) // max(1, int(slots)))
    return 0        # unknown: the caller decides what to do with an unbudgeted request


class Pool:
    """One loaded model's KV budget on one upstream.

    Weighted FIFO. `total` is the model instance's context, in tokens.
    """

    def __init__(self, total: int) -> None:
        self.total = int(total)
        self._used = 0
        self._cv = threading.Condition()
        self._queue: list[list] = []      # [[ticket, tokens, admitted]]
        self._next = 0
        self.admitted = 0
        self.held = 0                     # how many have ever had to wait
        self.rejected = 0

    # --- inspection --------------------------------------------------------
    @property
    def used(self) -> int:
        with self._cv:
            return self._used

    @property
    def free(self) -> int:
        with self._cv:
            return max(0, self.total - self._used)

    @property
    def waiting(self) -> int:
        with self._cv:
            return len(self._queue)

    def snapshot(self) -> dict:
        with self._cv:
            return {"total": self.total, "used": self._used,
                    "free": max(0, self.total - self._used),
                    "waiting": len(self._queue), "admitted": self.admitted,
                    "held": self.held, "rejected": self.rejected}

    # --- the control -------------------------------------------------------
    def acquire(self, tokens: int, timeout: float,
                cancelled=None) -> Lease | None:
        """Reserve `tokens`, waiting up to `timeout` seconds. None on timeout.

        `cancelled` is an optional callable: when it returns True the wait is
        abandoned. A caller that has gone away must stop holding a place in the
        line, or the queue fills with ghosts.
        """
        tokens = max(0, int(tokens))
        if tokens > self.total:
            # Refused here rather than queued: no amount of waiting makes room
            # that does not exist, and a caller deserves to be told so at once.
            with self._cv:
                self.rejected += 1
            raise TooLarge(f"{tokens} tokens exceeds the {self.total}-token pool")

        started = time.monotonic()
        deadline = started + max(0.0, timeout)
        with self._cv:
            ticket = self._next
            self._next += 1
            entry = [ticket, tokens, False]
            self._queue.append(entry)
            if len(self._queue) > 1 or self._used + tokens > self.total:
                self.held += 1
            while True:
                # Only the HEAD may be admitted. This is the whole of the fairness
                # guarantee: without it a stream of small requests walks past a
                # large one indefinitely.
                if self._queue and self._queue[0] is entry \
                        and self._used + tokens <= self.total:
                    self._used += tokens
                    self._queue.pop(0)
                    self.admitted += 1
                    self._cv.notify_all()
                    return Lease(tokens, time.monotonic() - started)
                remaining = deadline - time.monotonic()
                if remaining <= 0 or (cancelled is not None and cancelled()):
                    try:
                        self._queue.remove(entry)
                    except ValueError:
                        pass
                    # Whoever is now at the head may fit where this one did not.
                    self._cv.notify_all()
                    return None
                self._cv.wait(min(remaining, 0.25))

    def release(self, lease: Lease | None) -> None:
        if lease is None:
            return
        with self._cv:
            self._used = max(0, self._used - lease.tokens)
            self._cv.notify_all()


class Admission:
    """Pools keyed by (upstream, model). One loaded model, one KV allocation."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._pools: dict[tuple, Pool] = {}

    def pool(self, upstream: str, model: str, total: int) -> Pool:
        key = (upstream, model)
        with self._lock:
            p = self._pools.get(key)
            # A pool whose size has changed -- the model was reloaded with a
            # different context -- is replaced rather than resized: outstanding
            # leases belong to the old allocation and must drain against it.
            if p is None or p.total != int(total):
                p = Pool(total)
                self._pools[key] = p
            return p

    def snapshot(self) -> dict:
        with self._lock:
            return {f"{u}/{m}": p.snapshot() for (u, m), p in self._pools.items()}
