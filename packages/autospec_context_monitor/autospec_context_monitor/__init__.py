"""autospec-context-monitor — Python package for context-window monitoring.

Public API (importable from this top-level package):

- :class:`Usage` — frozen dataclass with token-usage snapshot.
- :class:`HarnessAdapter` — runtime-checkable Protocol every adapter must satisfy.
- :class:`TranscriptNotFoundError` — raised when a live transcript cannot be located.
"""
from .adapters.base import HarnessAdapter, TranscriptNotFoundError, Usage

__all__ = ["Usage", "HarnessAdapter", "TranscriptNotFoundError"]
