"""autospec_context_monitor.stats — append-only JSONL telemetry ledger.

Records rollover/compact events to ``~/.autospec/monitors/stats.jsonl``.
Never raises on disk errors; silently logs and returns.

Usage::

    from autospec_context_monitor.stats import record

    record("compact_fired", harness="claude", tmux_session="my-cc",
           pct=0.52, cwd="/home/user/myproject")
"""

from __future__ import annotations

import json
import logging
import os
import time
from pathlib import Path

_STATS_FILE = Path.home() / ".autospec" / "monitors" / "stats.jsonl"

_logger = logging.getLogger(__name__)


def record(event_type: str, **fields) -> None:  # noqa: ANN003
    """Append a single JSON event line to the stats ledger.

    Args:
        event_type: One of ``compact_fired``, ``rollover_fired``,
                    ``rollover_aborted``, ``handoff_invalid``.
        **fields:   Extra scalar fields merged into the record.
                    Typical keys: ``harness``, ``tmux_session``, ``pct``, ``cwd``.

    The record written has at minimum::

        {"ts": "<ISO-8601-UTC>", "event": "<event_type>", ...fields}

    Never raises: disk errors are logged at WARNING level and the function
    returns silently to avoid disrupting the monitor loop.
    """
    row: dict = {
        "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "event": event_type,
    }
    row.update(fields)

    try:
        _STATS_FILE.parent.mkdir(parents=True, exist_ok=True)
        # O_APPEND guarantees atomic single-write appends on POSIX.
        fd = os.open(str(_STATS_FILE), os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
        try:
            line = json.dumps(row) + "\n"
            os.write(fd, line.encode())
        finally:
            os.close(fd)
    except Exception as exc:  # noqa: BLE001
        _logger.warning("stats.record: failed to write event %r: %s", event_type, exc)
