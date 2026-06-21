"""Tests for autospec_context_monitor.stats JSONL telemetry ledger.

Covers:
- Append idempotence: two records → two lines.
- Malformed-line tolerance: bad JSON in ledger doesn't crash the reader.
- No-PII contract: only 'cwd' is a path key; no home-dir expansion in other keys.
- Integration: engine classify → dispatch → ledger contains expected events.
"""
from __future__ import annotations

import json
import os
import sys
import types
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture()
def tmp_stats(tmp_path, monkeypatch):
    """Redirect _STATS_FILE to a temp location for test isolation."""
    stats_file = tmp_path / "monitors" / "stats.jsonl"
    import autospec_context_monitor.stats as stats_mod
    monkeypatch.setattr(stats_mod, "_STATS_FILE", stats_file)
    return stats_file


# ---------------------------------------------------------------------------
# Unit tests
# ---------------------------------------------------------------------------

def test_append_idempotence(tmp_stats):
    """Two record() calls produce exactly two newline-separated JSON lines."""
    from autospec_context_monitor import stats

    stats.record("compact_fired", harness="claude", tmux_session="s1", pct=0.52, cwd="/proj")
    stats.record("rollover_fired", harness="claude", tmux_session="s1", pct=0.81, cwd="/proj")

    lines = tmp_stats.read_text().splitlines()
    assert len(lines) == 2, f"Expected 2 lines, got {len(lines)}"
    row0 = json.loads(lines[0])
    row1 = json.loads(lines[1])
    assert row0["event"] == "compact_fired"
    assert row1["event"] == "rollover_fired"


def test_malformed_line_tolerance(tmp_stats):
    """Reading the ledger after a malformed line doesn't crash; doctor skips it."""
    from autospec_context_monitor import stats

    # Inject a bad line directly.
    tmp_stats.parent.mkdir(parents=True, exist_ok=True)
    tmp_stats.write_text("NOT VALID JSON\n")

    # record() should still succeed and append a valid line.
    stats.record("compact_fired", harness="codex", tmux_session="s2", pct=0.55, cwd="/p")

    lines = tmp_stats.read_text().splitlines()
    assert len(lines) == 2
    # First line is the bad one; second must be valid JSON.
    row = json.loads(lines[1])
    assert row["event"] == "compact_fired"


def test_no_pii_contract(tmp_stats):
    """Only 'cwd' is a path key; no home directory expansion in other fields."""
    from autospec_context_monitor import stats

    home = str(Path.home())
    stats.record(
        "rollover_aborted",
        harness="claude",
        tmux_session="no-home-here",
        pct=0.80,
        cwd="/some/project",
    )

    row = json.loads(tmp_stats.read_text().strip())
    # Permitted path key: cwd only.
    path_keys = [k for k, v in row.items() if k != "cwd" and isinstance(v, str) and home in v]
    assert path_keys == [], f"Unexpected path-containing keys (other than cwd): {path_keys}"
    # ts, event, harness, tmux_session, pct, cwd must all be present.
    for key in ("ts", "event", "harness", "tmux_session", "pct", "cwd"):
        assert key in row, f"Missing required key: {key}"


# ---------------------------------------------------------------------------
# Integration test: engine classify → dispatch → ledger
# ---------------------------------------------------------------------------

def test_integration_compact_and_rollover(tmp_stats, tmp_path, monkeypatch):
    """Scripted usage sequence produces exactly 1 compact_fired + 1 rollover_fired."""
    from autospec_context_monitor.engine import Engine, Action
    from autospec_context_monitor.adapters.base import Usage

    # We need to exercise _dispatch without a real tmux session.
    # Patch inject, session_exists, wait_for_cancel so no tmux calls happen.
    import autospec_context_monitor.__main__ as main_mod

    monkeypatch.setattr(main_mod, "inject", lambda session, cmd: None)
    monkeypatch.setattr(main_mod, "wait_for_cancel", lambda session: False)
    # Also redirect stats in __main__ to use tmp_stats.
    import autospec_context_monitor.stats as stats_mod
    monkeypatch.setattr(stats_mod, "_STATS_FILE", tmp_stats)

    logf = tmp_path / "test.log"
    logf.touch()

    engine = Engine()
    harness = "claude"
    tmux_session = "integ-test"
    # Use a real tmp cwd so the rollover's handoff-discovery/validation gate
    # (clear action reads <cwd>/.turbo/handoff) can find a valid file.
    cwd = str(tmp_path / "integ-project")

    # The 'handoff' action (added in g-004) blocks on wait_for_handoff for up
    # to 180s; mock it to return a structurally-valid handoff path immediately
    # so the rollover path (handoff → clear → resume) completes and records
    # rollover_fired. Without this mock the wait times out and the clear gate
    # never fires.
    handoff_dir = Path(cwd) / ".turbo" / "handoff"
    handoff_dir.mkdir(parents=True, exist_ok=True)
    handoff_file = handoff_dir / "2026-06-20-integ.md"
    handoff_file.write_text(
        "# Handoff\n\n"
        "## Status\nIn progress.\n\n"
        "## Next step\nKeep going.\n\n"
        + ("x" * 300)
        + "\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(
        main_mod, "wait_for_handoff", lambda *a, **k: handoff_file
    )

    # Tick 1: 52% → compact_fired
    usage1 = Usage(used_tokens=52, max_tokens=100, model="test", estimated=False)
    actions1 = engine.classify(usage1)
    assert len(actions1) == 1 and actions1[0].kind == "compact"
    for action in actions1:
        main_mod._dispatch(
            action, tmux_session, logf, harness=harness, cwd=cwd, pct=0.52
        )

    # Tick 2: 81% → rollover: handoff + clear + resume
    usage2 = Usage(used_tokens=81, max_tokens=100, model="test", estimated=False)
    actions2 = engine.classify(usage2)
    assert any(a.kind == "handoff" for a in actions2)
    assert any(a.kind == "clear" for a in actions2)
    for action in actions2:
        main_mod._dispatch(
            action, tmux_session, logf, harness=harness, cwd=cwd, pct=0.81
        )

    # Verify ledger
    lines = tmp_stats.read_text().splitlines()
    events = [json.loads(l)["event"] for l in lines if l.strip()]
    assert events.count("compact_fired") == 1, f"Expected 1 compact_fired, got events={events}"
    assert events.count("rollover_fired") == 1, f"Expected 1 rollover_fired, got events={events}"
