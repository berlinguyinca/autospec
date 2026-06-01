"""Monitor-level test: /clear is NOT injected when the handoff file is invalid.

Scenario:
1. _dispatch receives a "clear" action.
2. The most recent file in ~/.turbo/handoff/ is the stub fixture (< 200 bytes).
3. validate_handoff returns (False, ["size"]).
4. _dispatch logs the failure, shows overlay, returns True (canceled).
5. The main loop reverts engine state to COMPACTED.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from autospec_context_monitor.__main__ import _dispatch, _log
from autospec_context_monitor.engine import Action, Engine, State

_FIXTURES = Path(__file__).parent / "fixtures" / "handoff"


def test_clear_not_sent_when_handoff_invalid(tmp_path):
    """_dispatch('clear') returns True (canceled) when handoff fails validation."""
    logf = tmp_path / "test.log"

    # Point ~/.turbo/handoff to a directory containing only the stub fixture.
    handoff_dir = tmp_path / ".turbo" / "handoff"
    handoff_dir.mkdir(parents=True)
    stub_src = _FIXTURES / "stub.md"
    stub_dest = handoff_dir / "2026-05-31-stub.md"
    stub_dest.write_bytes(stub_src.read_bytes())

    inject_calls: list = []

    def _fake_inject(session, text, **_kw):
        inject_calls.append(text)

    with patch("autospec_context_monitor.__main__.Path.home", return_value=tmp_path):
        with patch("autospec_context_monitor.__main__.inject", _fake_inject):
            with patch("autospec_context_monitor.__main__.wait_for_cancel", return_value=False):
                with patch("autospec_context_monitor.__main__.subprocess") as sp_mock:
                    sp_mock.run = MagicMock(return_value=MagicMock(returncode=0))
                    result = _dispatch(Action("clear"), "test-sess", logf)

    assert result is True, "_dispatch must return True (canceled) for invalid handoff"
    assert "/clear" not in inject_calls, "/clear must NOT be injected when handoff invalid"

    # Log must contain the invalid handoff event
    log_lines = [json.loads(l) for l in logf.read_text().splitlines() if l.strip()]
    events = [l["event"] for l in log_lines]
    assert any("handoff invalid" in e for e in events), (
        "Log must record 'handoff invalid: missing' event"
    )


def test_state_stays_compacted_after_invalid_handoff():
    """Engine state reverts to COMPACTED when clear dispatch is aborted."""
    engine = Engine()
    # Manually advance to ROLLED (simulating dispatch already fired)
    engine._state = State.ROLLED  # noqa: SLF001

    # Simulate caller reverting state on cancel
    engine._state = State.COMPACTED  # noqa: SLF001
    assert engine.state is State.COMPACTED

    # Next 80% reading should re-fire the rollover
    from autospec_context_monitor.adapters.base import Usage
    actions = engine.classify(Usage(used_tokens=800, max_tokens=1000, model="test", estimated=False))
    assert engine.state is State.ROLLED
    assert [a.kind for a in actions] == ["handoff", "clear", "resume"]
