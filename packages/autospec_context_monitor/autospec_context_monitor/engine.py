"""State machine engine for context-window monitoring.

Transitions:
  NORMAL → COMPACTED  when pct >= 0.50  (also taken on direct climb to >=0.80;
                                         compaction always runs first per spec)
  COMPACTED → ROLLED  when pct >= 0.80
  COMPACTED → NORMAL  when pct < 0.30   (compaction worked)
  ROLLED → NORMAL     when pct < 0.30   (new transcript detected after /clear)

Spec §Threshold state machine requires NORMAL → COMPACTED → ROLLED as the only
path: compaction must always be attempted before a rollover, since /compact
alone often drops pct below the rollover threshold.

Invariant: each transition fires at most once per state entry.
Re-firing requires returning to the prior state first.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum, auto

from .adapters.base import Usage


class State(Enum):
    NORMAL = auto()
    COMPACTED = auto()
    ROLLED = auto()


@dataclass
class Action:
    """A side-effect-free descriptor the *driver* executes."""

    kind: str  # "compact" | "handoff" | "clear" | "resume" | "noop"
    payload: str = ""


class Engine:
    """Pure state machine; caller executes returned Actions.

    >>> from autospec_context_monitor.adapters.base import Usage
    >>> e = Engine()
    >>> e.classify(Usage(used_tokens=0, max_tokens=100, model="test", estimated=False))
    []
    """

    def __init__(self) -> None:
        self._state: State = State.NORMAL

    @property
    def state(self) -> State:
        return self._state

    def reset(self) -> None:
        """Reset state to NORMAL (called when a new transcript is detected)."""
        self._state = State.NORMAL

    def classify(self, usage: Usage) -> list[Action]:
        """Return Actions that should be executed for *usage*.

        The method is *pure* with respect to external I/O — it only
        mutates internal state and returns Action records.
        No transition fires more than once per state entry.
        """
        pct = usage.used_tokens / usage.max_tokens

        if self._state is State.NORMAL:
            # Spec §Threshold state machine: NORMAL never transitions
            # directly to ROLLED. On a direct climb to >= 0.80 we still
            # enter COMPACTED and emit [compact]; the next tick (still
            # >= 0.80 because compaction hasn't taken effect yet) will
            # transition COMPACTED -> ROLLED. This guarantees /compact is
            # always attempted before a rollover, which often makes the
            # rollover unnecessary.
            if pct >= 0.50:
                self._state = State.COMPACTED
                return [Action("compact")]

        elif self._state is State.COMPACTED:
            if pct >= 0.80:
                self._state = State.ROLLED
                return [Action("handoff"), Action("clear"), Action("resume")]
            if pct < 0.30:
                self._state = State.NORMAL
                return [Action("noop", "reset:compacted->normal")]

        elif self._state is State.ROLLED:
            if pct < 0.30:
                self._state = State.NORMAL
                return [Action("noop", "reset:rolled->normal")]

        return []
