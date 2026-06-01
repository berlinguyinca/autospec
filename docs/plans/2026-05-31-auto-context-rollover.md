# Auto Context Rollover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a tmux-wrapped monitor that auto-`/compact`s at 50% context and writes-handoff + `/clear`s + resumes at 80% across `claude`, `codex`, and `opencode` sessions.

**Architecture:** A bash launcher (`autospec-session`) wraps the harness inside tmux and spawns a Python monitor daemon. The daemon polls a per-harness adapter for `(used_tokens, max_tokens)`, runs a 3-state machine (NORMAL → COMPACTED → ROLLED), and injects slash commands back into the running harness via `tmux send-keys`. Opt-in via `install.sh`, with shell function shims (`claude()`, `codex()`, `opencode()`) routing through the launcher.

**Tech Stack:** Python 3.11+ (stdlib only — no third-party deps), bash 4+, tmux 3+, pytest.

**Spec:** `docs/specs/2026-05-31-auto-context-rollover-design.md`

---

## File Structure

**New python package** at `packages/autospec_context_monitor/`:
- `__init__.py` — version constant
- `engine.py` — `Engine` class (state machine)
- `injector.py` — `inject()`, `pane_ready()`, `session_exists()`
- `handoff.py` — `wait_for_handoff(dir, since) -> Path`
- `doctor.py` — `--doctor` implementation
- `monitor.py` — daemon entrypoint (`python -m autospec_context_monitor`)
- `models.py` — `Usage` dataclass + model→max-tokens lookup tables
- `adapters/__init__.py` — `load(name) -> HarnessAdapter`
- `adapters/base.py` — `HarnessAdapter` Protocol + `TranscriptNotFoundError`
- `adapters/claude.py`
- `adapters/codex.py`
- `adapters/opencode.py`

**New scripts** at `scripts/`:
- `autospec-session` — bash launcher
- `autospec-context-monitor` — thin python shim (`exec python -m autospec_context_monitor "$@"`)

**New skill** at `skills/autospec-rollover-status/`:
- `SKILL.md`

**Tests** at `tests/context_monitor/`:
- `adapters/test_claude.py`, `test_codex.py`, `test_opencode.py`
- `test_engine.py`, `test_injector.py`, `test_handoff.py`
- `e2e/test_rollover_claude.py`, `test_rollover_codex.py`, `test_rollover_opencode.py`
- `fixtures/claude/{short-session,sonnet-1m,missing-usage}.jsonl`
- `fixtures/codex/{info-populated,info-null}.jsonl`
- `fixtures/opencode/{session.db,schema-changed.db}`
- `conftest.py` — shared `tmp_tmux_session` fixture, time monkey-patches

**Modified:**
- `install.sh` — add `prompt_user_for_auto_rollover()` + shell-rc block writer
- `uninstall.sh` — remove the block
- `README.md` — new "Auto context rollover" section
- `AGENTS.md` — one-line pointer

Files split by responsibility, not layer. Each ≤300 LOC target. State machine, injector, handoff polling, and adapters are independently testable.

---

## Task 1: Bootstrap python package + test harness

**Files:**
- Create: `packages/autospec_context_monitor/pyproject.toml`
- Create: `packages/autospec_context_monitor/autospec_context_monitor/__init__.py`
- Create: `tests/context_monitor/__init__.py`
- Create: `tests/context_monitor/conftest.py`
- Modify: `scripts/test.sh` or top-level test runner (verify which exists; if neither, add minimal `tests/context_monitor/README.md` with run command)

- [ ] **Step 1: Verify python + pytest available**

Run: `python3 --version && python3 -m pytest --version`
Expected: Python ≥ 3.11; pytest version printed. If pytest missing, `python3 -m pip install --user pytest`.

- [ ] **Step 2: Create `pyproject.toml`**

```toml
[project]
name = "autospec-context-monitor"
version = "0.1.0"
requires-python = ">=3.11"
description = "Auto context rollover for claude/codex/opencode sessions"
dependencies = []  # stdlib only

[project.scripts]
autospec-context-monitor = "autospec_context_monitor.monitor:main"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[tool.setuptools.packages.find]
where = ["."]
```

- [ ] **Step 3: Create `__init__.py`**

```python
__version__ = "0.1.0"
```

- [ ] **Step 4: Create `tests/context_monitor/conftest.py`**

```python
import os
import subprocess
import uuid
import pytest


@pytest.fixture
def tmp_tmux_session():
    """Create a throwaway tmux session, yield its name, tear down."""
    name = f"as-test-{uuid.uuid4().hex[:8]}"
    subprocess.run(
        ["tmux", "new-session", "-d", "-s", name, "bash"],
        check=True,
    )
    yield name
    subprocess.run(["tmux", "kill-session", "-t", name], check=False)


@pytest.fixture
def fast_sleep(monkeypatch):
    """Replace time.sleep with a no-op in tests."""
    import time
    monkeypatch.setattr(time, "sleep", lambda _s: None)
```

- [ ] **Step 5: Smoke-test the harness**

Run: `cd packages/autospec_context_monitor && python3 -m pytest ../../tests/context_monitor -v`
Expected: `no tests ran` (or `0 collected`) without errors.

- [ ] **Step 6: Commit**

```bash
git add packages/autospec_context_monitor/ tests/context_monitor/
git commit -m "feat(context-monitor): bootstrap python package skeleton"
```

---

## Task 2: Adapter base contract + Usage dataclass

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/models.py`
- Create: `packages/autospec_context_monitor/autospec_context_monitor/adapters/__init__.py`
- Create: `packages/autospec_context_monitor/autospec_context_monitor/adapters/base.py`
- Test: `tests/context_monitor/test_models.py`

- [ ] **Step 1: Write the failing test for Usage**

```python
# tests/context_monitor/test_models.py
from autospec_context_monitor.models import Usage


def test_usage_pct():
    u = Usage(used_tokens=50_000, max_tokens=200_000, model="claude-sonnet-4-6", estimated=False)
    assert u.pct == 25.0


def test_usage_pct_handles_zero_max():
    u = Usage(used_tokens=10, max_tokens=0, model="unknown", estimated=True)
    assert u.pct == 0.0
```

- [ ] **Step 2: Run test, expect ImportError**

Run: `python3 -m pytest tests/context_monitor/test_models.py -v`
Expected: FAIL — `ModuleNotFoundError: autospec_context_monitor`

- [ ] **Step 3: Install the package in editable mode**

Run: `cd packages/autospec_context_monitor && python3 -m pip install --user -e .`

- [ ] **Step 4: Write `models.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/models.py
from dataclasses import dataclass


CLAUDE_MAX_TOKENS = {
    "claude-sonnet-4-6": 200_000,
    "claude-opus-4-7": 200_000,
    "claude-opus-4-7[1m]": 1_000_000,
    "claude-haiku-4-5-20251001": 200_000,
}

CODEX_MAX_TOKENS = {
    "gpt-5": 400_000,
    "o3": 200_000,
    "o4-mini": 200_000,
}

OPENCODE_MAX_TOKENS = {
    # Filled from models_cache.json at runtime when available
}

DEFAULT_MAX_TOKENS = 200_000


@dataclass(frozen=True)
class Usage:
    used_tokens: int
    max_tokens: int
    model: str
    estimated: bool

    @property
    def pct(self) -> float:
        if self.max_tokens <= 0:
            return 0.0
        return (self.used_tokens / self.max_tokens) * 100.0
```

- [ ] **Step 5: Rerun test, expect PASS**

Run: `python3 -m pytest tests/context_monitor/test_models.py -v`
Expected: 2 passed.

- [ ] **Step 6: Write `adapters/base.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/adapters/base.py
from pathlib import Path
from typing import Literal, Protocol

from autospec_context_monitor.models import Usage


class TranscriptNotFoundError(RuntimeError):
    """Raised when no transcript file matches the session hint within the wait window."""


LogicalCommand = Literal["clear", "compact", "handoff"]


class HarnessAdapter(Protocol):
    name: str

    def find_transcript(self, hint: dict) -> Path: ...
    def read_usage(self, transcript: Path) -> Usage: ...
    def command(self, logical: LogicalCommand) -> str: ...
```

- [ ] **Step 7: Write `adapters/__init__.py` with `load(name)`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/adapters/__init__.py
from importlib import import_module

from autospec_context_monitor.adapters.base import HarnessAdapter


_SUPPORTED = {"claude", "codex", "opencode"}


def load(name: str) -> HarnessAdapter:
    if name not in _SUPPORTED:
        raise ValueError(f"unknown harness: {name!r} (supported: {sorted(_SUPPORTED)})")
    mod = import_module(f"autospec_context_monitor.adapters.{name}")
    return mod.Adapter()
```

- [ ] **Step 8: Write the failing test for `load()`**

```python
# tests/context_monitor/test_models.py — add at bottom
import pytest
from autospec_context_monitor.adapters import load


def test_load_unknown_harness():
    with pytest.raises(ValueError, match="unknown harness"):
        load("hypothetical")
```

- [ ] **Step 9: Run, expect PASS**

Run: `python3 -m pytest tests/context_monitor/test_models.py -v`
Expected: 3 passed.

- [ ] **Step 10: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/models.py \
        packages/autospec_context_monitor/autospec_context_monitor/adapters/__init__.py \
        packages/autospec_context_monitor/autospec_context_monitor/adapters/base.py \
        tests/context_monitor/test_models.py
git commit -m "feat(context-monitor): adapter contract + Usage model"
```

---

## Task 3: Claude adapter

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/adapters/claude.py`
- Test: `tests/context_monitor/adapters/test_claude.py`
- Test fixtures: `tests/context_monitor/fixtures/claude/short-session.jsonl`, `sonnet-1m.jsonl`, `missing-usage.jsonl`

- [ ] **Step 1: Create real fixtures**

Capture a real CC transcript snippet (find a small `.jsonl` under `~/.claude/projects/`) and copy 3 turns of it to `tests/context_monitor/fixtures/claude/short-session.jsonl`. Redact any PII. Make `sonnet-1m.jsonl` a one-turn file with `message.model = "claude-opus-4-7[1m]"` and a usage block. Make `missing-usage.jsonl` a one-turn assistant message with no `usage` key.

If real fixtures aren't available, hand-author minimal ones following this shape:

```json
{"type":"assistant","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}
```

- [ ] **Step 2: Write failing tests**

```python
# tests/context_monitor/adapters/__init__.py — empty file
# tests/context_monitor/adapters/test_claude.py
from pathlib import Path

import pytest

from autospec_context_monitor.adapters import load

FIXTURES = Path(__file__).parent.parent / "fixtures" / "claude"


def test_read_usage_sums_input_and_output():
    adapter = load("claude")
    usage = adapter.read_usage(FIXTURES / "short-session.jsonl")
    assert usage.used_tokens > 0
    assert usage.max_tokens == 200_000
    assert usage.estimated is False


def test_read_usage_recognizes_1m_suffix():
    adapter = load("claude")
    usage = adapter.read_usage(FIXTURES / "sonnet-1m.jsonl")
    assert usage.max_tokens == 1_000_000


def test_read_usage_handles_missing_usage_block():
    adapter = load("claude")
    usage = adapter.read_usage(FIXTURES / "missing-usage.jsonl")
    assert usage.used_tokens == 0
    assert usage.estimated is False  # CC always reports usage when it exists


def test_command_mappings():
    adapter = load("claude")
    assert adapter.command("clear") == "/clear"
    assert adapter.command("compact") == "/compact"
    assert "/create-handoff" in adapter.command("handoff")
```

- [ ] **Step 3: Run, expect ModuleNotFoundError**

Run: `python3 -m pytest tests/context_monitor/adapters/test_claude.py -v`
Expected: collection error or 4 fails.

- [ ] **Step 4: Implement `claude.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/adapters/claude.py
import json
import os
import time
from pathlib import Path

from autospec_context_monitor.adapters.base import HarnessAdapter, TranscriptNotFoundError
from autospec_context_monitor.models import CLAUDE_MAX_TOKENS, DEFAULT_MAX_TOKENS, Usage


def _cwd_slug(cwd: str) -> str:
    """Claude Code project dir is the cwd with / → -."""
    return "-" + cwd.replace("/", "-").lstrip("-")


class Adapter:
    name = "claude"

    def find_transcript(self, hint: dict) -> Path:
        cwd = hint["cwd"]
        started_at = hint.get("started_at", 0)
        project_dir = Path.home() / ".claude" / "projects" / _cwd_slug(cwd)
        deadline = time.time() + 30
        while time.time() < deadline:
            if project_dir.exists():
                candidates = [
                    p for p in project_dir.glob("*.jsonl")
                    if p.stat().st_mtime >= started_at
                ]
                if candidates:
                    return max(candidates, key=lambda p: p.stat().st_mtime)
            time.sleep(1)
        raise TranscriptNotFoundError(f"no jsonl under {project_dir} since {started_at}")

    def read_usage(self, transcript: Path) -> Usage:
        used = 0
        model = "claude-sonnet-4-6"
        for line in transcript.read_text().splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if rec.get("type") != "assistant":
                continue
            msg = rec.get("message", {})
            model = msg.get("model", model)
            u = msg.get("usage")
            if u:
                used += u.get("input_tokens", 0) + u.get("output_tokens", 0)
        max_tokens = CLAUDE_MAX_TOKENS.get(model, DEFAULT_MAX_TOKENS)
        return Usage(used_tokens=used, max_tokens=max_tokens, model=model, estimated=False)

    def command(self, logical: str) -> str:
        return {
            "clear": "/clear",
            "compact": "/compact",
            "handoff": "Please run /create-handoff and wait for the file to be written before responding further.",
        }[logical]
```

- [ ] **Step 5: Run tests, expect PASS**

Run: `python3 -m pytest tests/context_monitor/adapters/test_claude.py -v`
Expected: 4 passed.

- [ ] **Step 6: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/adapters/claude.py \
        tests/context_monitor/fixtures/claude/ \
        tests/context_monitor/adapters/test_claude.py \
        tests/context_monitor/adapters/__init__.py
git commit -m "feat(context-monitor): Claude Code adapter"
```

---

## Task 4: Codex adapter (with info:null fallback)

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/adapters/codex.py`
- Test: `tests/context_monitor/adapters/test_codex.py`
- Fixtures: `tests/context_monitor/fixtures/codex/{info-populated,info-null}.jsonl`

- [ ] **Step 1: Author fixtures**

`info-populated.jsonl`:
```json
{"type":"event_msg","payload":{"type":"token_count","info":{"input_tokens":2000,"output_tokens":800}}}
```

`info-null.jsonl` (older codex versions):
```json
{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":12.5}}}}
{"type":"message","payload":{"role":"user","content":"hello world this is some text"}}
{"type":"message","payload":{"role":"assistant","content":"hi there"}}
```

- [ ] **Step 2: Write failing tests**

```python
# tests/context_monitor/adapters/test_codex.py
from pathlib import Path
from autospec_context_monitor.adapters import load

FIXTURES = Path(__file__).parent.parent / "fixtures" / "codex"


def test_info_populated_uses_explicit_tokens():
    adapter = load("codex")
    usage = adapter.read_usage(FIXTURES / "info-populated.jsonl")
    assert usage.used_tokens == 2800
    assert usage.estimated is False


def test_info_null_falls_back_to_text_estimate():
    adapter = load("codex")
    usage = adapter.read_usage(FIXTURES / "info-null.jsonl")
    # Two messages, ~40 chars combined → ~11 tokens via /3.5
    assert usage.estimated is True
    assert 5 <= usage.used_tokens <= 20


def test_command_mappings():
    adapter = load("codex")
    assert adapter.command("clear") == "/new"
    assert adapter.command("compact") == "/compact"
    assert "handoff" in adapter.command("handoff").lower()
```

- [ ] **Step 3: Run, expect fails**

Run: `python3 -m pytest tests/context_monitor/adapters/test_codex.py -v`
Expected: fails.

- [ ] **Step 4: Implement `codex.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/adapters/codex.py
import json
import time
from pathlib import Path

from autospec_context_monitor.adapters.base import HarnessAdapter, TranscriptNotFoundError
from autospec_context_monitor.models import CODEX_MAX_TOKENS, DEFAULT_MAX_TOKENS, Usage


CHARS_PER_TOKEN = 3.5


class Adapter:
    name = "codex"

    def find_transcript(self, hint: dict) -> Path:
        started_at = hint.get("started_at", 0)
        root = Path.home() / ".codex" / "sessions"
        deadline = time.time() + 30
        while time.time() < deadline:
            if root.exists():
                candidates = [
                    p for p in root.rglob("rollout-*.jsonl")
                    if p.stat().st_mtime >= started_at
                ]
                if candidates:
                    return max(candidates, key=lambda p: p.stat().st_mtime)
            time.sleep(1)
        raise TranscriptNotFoundError(f"no rollout-*.jsonl under {root} since {started_at}")

    def read_usage(self, transcript: Path) -> Usage:
        explicit_tokens = 0
        have_explicit = False
        text_chars = 0
        model = "gpt-5"
        for line in transcript.read_text().splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            payload = rec.get("payload", {}) or {}
            if rec.get("type") == "event_msg" and payload.get("type") == "token_count":
                info = payload.get("info")
                if info:
                    explicit_tokens = info.get("input_tokens", 0) + info.get("output_tokens", 0)
                    have_explicit = True
            elif rec.get("type") == "message":
                content = payload.get("content", "")
                if isinstance(content, str):
                    text_chars += len(content)
        if have_explicit:
            used, estimated = explicit_tokens, False
        else:
            used, estimated = int(text_chars / CHARS_PER_TOKEN), True
        max_tokens = CODEX_MAX_TOKENS.get(model, DEFAULT_MAX_TOKENS)
        return Usage(used_tokens=used, max_tokens=max_tokens, model=model, estimated=estimated)

    def command(self, logical: str) -> str:
        return {
            "clear": "/new",
            "compact": "/compact",
            "handoff": (
                "Write a handoff file to .turbo/handoff/<YYYY-MM-DD>-<slug>.md "
                "capturing current task, status, open decisions, in-flight changes, "
                "and next step. Confirm the path when done."
            ),
        }[logical]
```

- [ ] **Step 5: Run, expect PASS**

Run: `python3 -m pytest tests/context_monitor/adapters/test_codex.py -v`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/adapters/codex.py \
        tests/context_monitor/fixtures/codex/ \
        tests/context_monitor/adapters/test_codex.py
git commit -m "feat(context-monitor): Codex CLI adapter with info:null fallback"
```

---

## Task 5: OpenCode adapter (SQLite polling)

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/adapters/opencode.py`
- Test: `tests/context_monitor/adapters/test_opencode.py`
- Fixtures: `tests/context_monitor/fixtures/opencode/{session.db,schema-changed.db}`

- [ ] **Step 1: Build the fixture SQLite db**

Run this once to create `session.db`:

```python
# scripts/build-opencode-fixture.py (one-shot, can be deleted after)
import sqlite3
from pathlib import Path

p = Path("tests/context_monitor/fixtures/opencode/session.db")
p.parent.mkdir(parents=True, exist_ok=True)
p.unlink(missing_ok=True)

con = sqlite3.connect(p)
con.executescript("""
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    started_at INTEGER
);
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    role TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER
);
INSERT INTO sessions VALUES ('s1', 1700000000);
INSERT INTO messages VALUES ('m1', 's1', 'user',      1500, 0);
INSERT INTO messages VALUES ('m2', 's1', 'assistant', 0, 800);
""")
con.commit()
con.close()
print("wrote", p)
```

Run: `python3 scripts/build-opencode-fixture.py`. Then for `schema-changed.db`, repeat with a different column name (`tokens_in` instead of `input_tokens`) — the adapter should fall back gracefully.

- [ ] **Step 2: Write failing tests**

```python
# tests/context_monitor/adapters/test_opencode.py
from pathlib import Path
from autospec_context_monitor.adapters import load

FIXTURES = Path(__file__).parent.parent / "fixtures" / "opencode"


def test_read_usage_sums_from_sqlite():
    adapter = load("opencode")
    usage = adapter.read_usage(FIXTURES / "session.db")
    assert usage.used_tokens == 2300
    assert usage.estimated is False


def test_read_usage_handles_schema_mismatch():
    adapter = load("opencode")
    usage = adapter.read_usage(FIXTURES / "schema-changed.db")
    # Falls back to estimated=True with used=0 when schema unknown
    assert usage.estimated is True


def test_command_mappings():
    adapter = load("opencode")
    assert adapter.command("clear") == "/new"
    assert adapter.command("compact") == "/compact"
```

- [ ] **Step 3: Run, expect fails**

Run: `python3 -m pytest tests/context_monitor/adapters/test_opencode.py -v`
Expected: fails.

- [ ] **Step 4: Implement `opencode.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/adapters/opencode.py
import sqlite3
import time
from pathlib import Path

from autospec_context_monitor.adapters.base import HarnessAdapter, TranscriptNotFoundError
from autospec_context_monitor.models import DEFAULT_MAX_TOKENS, OPENCODE_MAX_TOKENS, Usage


DB_PATH = Path.home() / ".local" / "share" / "opencode" / "opencode.db"


class Adapter:
    name = "opencode"

    def find_transcript(self, hint: dict) -> Path:
        # OpenCode stores everything in one db; the "transcript" is the db file.
        deadline = time.time() + 30
        while time.time() < deadline:
            if DB_PATH.exists():
                return DB_PATH
            time.sleep(1)
        raise TranscriptNotFoundError(f"opencode db not found at {DB_PATH}")

    def read_usage(self, transcript: Path) -> Usage:
        try:
            uri = f"file:{transcript}?mode=ro"
            con = sqlite3.connect(uri, uri=True)
            cur = con.execute(
                "SELECT COALESCE(SUM(input_tokens + output_tokens), 0) FROM messages"
            )
            (used,) = cur.fetchone()
            con.close()
            estimated = False
        except sqlite3.OperationalError:
            used, estimated = 0, True
        max_tokens = OPENCODE_MAX_TOKENS.get("default", DEFAULT_MAX_TOKENS)
        return Usage(used_tokens=used, max_tokens=max_tokens, model="opencode-default", estimated=estimated)

    def command(self, logical: str) -> str:
        return {
            "clear": "/new",
            "compact": "/compact",
            "handoff": (
                "Write a handoff file to .turbo/handoff/<YYYY-MM-DD>-<slug>.md "
                "capturing current task, status, open decisions, in-flight changes, "
                "and next step. Confirm the path when done."
            ),
        }[logical]
```

- [ ] **Step 5: Run tests, expect PASS**

Run: `python3 -m pytest tests/context_monitor/adapters/test_opencode.py -v`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/adapters/opencode.py \
        tests/context_monitor/fixtures/opencode/ \
        tests/context_monitor/adapters/test_opencode.py \
        scripts/build-opencode-fixture.py
git commit -m "feat(context-monitor): OpenCode SQLite adapter with schema fallback"
```

---

## Task 6: Injector (tmux send-keys + guards)

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/injector.py`
- Test: `tests/context_monitor/test_injector.py`

- [ ] **Step 1: Write failing tests**

```python
# tests/context_monitor/test_injector.py
import pytest
from autospec_context_monitor.injector import (
    inject, session_exists, InjectionError,
)


def test_session_exists_true(tmp_tmux_session):
    assert session_exists(tmp_tmux_session)


def test_session_exists_false():
    assert not session_exists("nonexistent-tmux-session-xyz")


def test_inject_writes_to_session(tmp_tmux_session):
    inject(tmp_tmux_session, "echo hello-from-injector", submit=True)
    # Capture pane and verify text appeared
    import subprocess, time
    time.sleep(0.5)
    out = subprocess.check_output(
        ["tmux", "capture-pane", "-p", "-t", tmp_tmux_session]
    ).decode()
    assert "hello-from-injector" in out


def test_inject_raises_on_missing_session():
    with pytest.raises(InjectionError):
        inject("nonexistent-tmux-session-xyz", "anything")
```

- [ ] **Step 2: Run tests, expect ImportError**

Run: `python3 -m pytest tests/context_monitor/test_injector.py -v`
Expected: fails.

- [ ] **Step 3: Implement `injector.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/injector.py
import subprocess
import time


class InjectionError(RuntimeError):
    pass


def session_exists(tmux_session: str) -> bool:
    r = subprocess.run(
        ["tmux", "has-session", "-t", tmux_session],
        capture_output=True,
    )
    return r.returncode == 0


def capture_pane(tmux_session: str, lines: int = 3) -> str:
    r = subprocess.run(
        ["tmux", "capture-pane", "-p", "-t", tmux_session],
        capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        return ""
    return "\n".join(r.stdout.splitlines()[-lines:])


def pane_ready(tmux_session: str, markers: tuple[str, ...]) -> bool:
    """True if the last line of the pane ends with any of the input markers."""
    tail = capture_pane(tmux_session, lines=1).rstrip()
    return any(tail.endswith(m) for m in markers)


def inject(
    tmux_session: str,
    text: str,
    *,
    submit: bool = True,
    retries: int = 1,
) -> None:
    if not session_exists(tmux_session):
        raise InjectionError(f"tmux session {tmux_session!r} not found")
    attempt = 0
    while True:
        try:
            subprocess.run(
                ["tmux", "send-keys", "-l", "-t", tmux_session, text],
                check=True, timeout=5,
            )
            if submit:
                subprocess.run(
                    ["tmux", "send-keys", "-t", tmux_session, "Enter"],
                    check=True, timeout=5,
                )
            return
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
            attempt += 1
            if attempt > retries:
                raise InjectionError(f"tmux send-keys failed: {e}") from e
            time.sleep(5)
```

- [ ] **Step 4: Run, expect 4 passed**

Run: `python3 -m pytest tests/context_monitor/test_injector.py -v`
Expected: 4 passed. (Requires tmux installed; skip on CI without tmux.)

- [ ] **Step 5: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/injector.py \
        tests/context_monitor/test_injector.py
git commit -m "feat(context-monitor): tmux injector with session/pane guards"
```

---

## Task 7: Handoff file polling

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/handoff.py`
- Test: `tests/context_monitor/test_handoff.py`

- [ ] **Step 1: Write failing test**

```python
# tests/context_monitor/test_handoff.py
import threading
import time
from pathlib import Path

import pytest

from autospec_context_monitor.handoff import HandoffTimeout, wait_for_handoff


def test_wait_returns_newest_file(tmp_path):
    since = time.time()
    target = tmp_path / "2026-05-31-test.md"

    def writer():
        time.sleep(0.2)
        target.write_text("# handoff")

    t = threading.Thread(target=writer)
    t.start()
    found = wait_for_handoff(tmp_path, since=since, timeout=2.0, poll_interval=0.05)
    t.join()
    assert found == target


def test_wait_times_out(tmp_path):
    since = time.time()
    with pytest.raises(HandoffTimeout):
        wait_for_handoff(tmp_path, since=since, timeout=0.3, poll_interval=0.05)


def test_ignores_files_older_than_since(tmp_path):
    old = tmp_path / "2026-05-30-old.md"
    old.write_text("old")
    since = time.time() + 0.1  # cutoff in the near future
    with pytest.raises(HandoffTimeout):
        wait_for_handoff(tmp_path, since=since, timeout=0.3, poll_interval=0.05)
```

- [ ] **Step 2: Run, expect ImportError**

Run: `python3 -m pytest tests/context_monitor/test_handoff.py -v`
Expected: fails.

- [ ] **Step 3: Implement `handoff.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/handoff.py
import time
from pathlib import Path


class HandoffTimeout(RuntimeError):
    pass


def wait_for_handoff(
    handoff_dir: Path,
    *,
    since: float,
    timeout: float = 180.0,
    poll_interval: float = 2.0,
) -> Path:
    """Block until a `.md` file with mtime >= since appears in handoff_dir.
    Returns the newest matching file. Raises HandoffTimeout if none appears."""
    handoff_dir = Path(handoff_dir)
    deadline = time.time() + timeout
    while time.time() < deadline:
        if handoff_dir.exists():
            candidates = [
                p for p in handoff_dir.glob("*.md")
                if p.stat().st_mtime >= since
            ]
            if candidates:
                return max(candidates, key=lambda p: p.stat().st_mtime)
        time.sleep(poll_interval)
    raise HandoffTimeout(f"no handoff appeared in {handoff_dir} within {timeout}s")
```

- [ ] **Step 4: Run, expect 3 passed**

Run: `python3 -m pytest tests/context_monitor/test_handoff.py -v`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/handoff.py \
        tests/context_monitor/test_handoff.py
git commit -m "feat(context-monitor): handoff file polling"
```

---

## Task 8: State machine engine

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/engine.py`
- Test: `tests/context_monitor/test_engine.py`

- [ ] **Step 1: Write failing tests**

```python
# tests/context_monitor/test_engine.py
from autospec_context_monitor.engine import Engine, State
from autospec_context_monitor.models import Usage


def make_usage(pct: float, max_tokens: int = 200_000) -> Usage:
    return Usage(
        used_tokens=int(pct / 100 * max_tokens),
        max_tokens=max_tokens,
        model="claude-sonnet-4-6",
        estimated=False,
    )


class FakeActions:
    def __init__(self):
        self.calls = []

    def compact(self):       self.calls.append("compact")
    def handoff_and_clear(self): self.calls.append("rollover")


def test_starts_normal():
    e = Engine(FakeActions())
    assert e.state == State.NORMAL


def test_50_pct_triggers_compact():
    actions = FakeActions()
    e = Engine(actions)
    e.tick(make_usage(40))
    assert actions.calls == []
    e.tick(make_usage(51))
    assert actions.calls == ["compact"]
    assert e.state == State.COMPACTED


def test_compact_does_not_refire_without_reset():
    actions = FakeActions()
    e = Engine(actions)
    e.tick(make_usage(51))
    e.tick(make_usage(55))
    e.tick(make_usage(60))
    assert actions.calls == ["compact"]


def test_drops_below_30_resets_to_normal():
    actions = FakeActions()
    e = Engine(actions)
    e.tick(make_usage(51))
    e.tick(make_usage(25))
    assert e.state == State.NORMAL
    e.tick(make_usage(52))
    assert actions.calls == ["compact", "compact"]


def test_80_pct_triggers_rollover():
    actions = FakeActions()
    e = Engine(actions)
    e.tick(make_usage(51))                  # COMPACTED
    e.tick(make_usage(81))                  # ROLLED
    assert actions.calls == ["compact", "rollover"]
    assert e.state == State.ROLLED


def test_rolled_back_to_normal_on_low_pct():
    actions = FakeActions()
    e = Engine(actions)
    e.tick(make_usage(51)); e.tick(make_usage(81))
    e.tick(make_usage(5))                   # new transcript, low usage
    assert e.state == State.NORMAL
```

- [ ] **Step 2: Run, expect fails**

Run: `python3 -m pytest tests/context_monitor/test_engine.py -v`
Expected: fails.

- [ ] **Step 3: Implement `engine.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/engine.py
from enum import Enum
from typing import Protocol

from autospec_context_monitor.models import Usage


COMPACT_AT = 50.0
ROLLOVER_AT = 80.0
RESET_BELOW = 30.0


class State(str, Enum):
    NORMAL = "normal"
    COMPACTED = "compacted"
    ROLLED = "rolled"


class Actions(Protocol):
    def compact(self) -> None: ...
    def handoff_and_clear(self) -> None: ...


class Engine:
    def __init__(self, actions: Actions):
        self.actions = actions
        self.state = State.NORMAL

    def tick(self, usage: Usage) -> None:
        pct = usage.pct
        if pct < RESET_BELOW and self.state != State.NORMAL:
            self.state = State.NORMAL
            return
        if self.state == State.NORMAL and pct >= COMPACT_AT:
            self.actions.compact()
            self.state = State.COMPACTED
            return
        if self.state == State.COMPACTED and pct >= ROLLOVER_AT:
            self.actions.handoff_and_clear()
            self.state = State.ROLLED
            return
```

- [ ] **Step 4: Run, expect 6 passed**

Run: `python3 -m pytest tests/context_monitor/test_engine.py -v`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/engine.py \
        tests/context_monitor/test_engine.py
git commit -m "feat(context-monitor): threshold state machine"
```

---

## Task 9: Monitor daemon entrypoint

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/monitor.py`
- Test: `tests/context_monitor/test_monitor.py`
- Create: `scripts/autospec-context-monitor`

- [ ] **Step 1: Write failing test for `RealActions`**

```python
# tests/context_monitor/test_monitor.py
from pathlib import Path
from autospec_context_monitor.monitor import RealActions


def test_real_actions_compact_calls_inject(monkeypatch, tmp_path):
    calls = []
    def fake_inject(sess, text, submit=True):
        calls.append((sess, text, submit))
    monkeypatch.setattr("autospec_context_monitor.monitor.inject", fake_inject)

    class FakeAdapter:
        def command(self, logical): return f"<{logical}>"

    actions = RealActions(
        tmux_session="testsess",
        adapter=FakeAdapter(),
        handoff_dir=tmp_path,
        started_at=0,
    )
    actions.compact()
    assert calls == [("testsess", "<compact>", True)]
```

- [ ] **Step 2: Implement `monitor.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/monitor.py
import argparse
import logging
import os
import sys
import time
from pathlib import Path

from autospec_context_monitor.adapters import load
from autospec_context_monitor.engine import Engine
from autospec_context_monitor.handoff import HandoffTimeout, wait_for_handoff
from autospec_context_monitor.injector import (
    InjectionError, inject, session_exists,
)


POLL_INTERVAL_S = 15
COMPACT_WAIT_S = 60
HANDOFF_WAIT_S = 180
KILL_FLAG = Path.home() / ".autospec" / "no-auto-rollover.flag"


class RealActions:
    def __init__(self, tmux_session: str, adapter, handoff_dir: Path, started_at: float):
        self.tmux_session = tmux_session
        self.adapter = adapter
        self.handoff_dir = handoff_dir
        self.started_at = started_at
        self.log = logging.getLogger("context-monitor.actions")

    def compact(self) -> None:
        cmd = self.adapter.command("compact")
        self.log.info("injecting compact: %r", cmd)
        inject(self.tmux_session, cmd, submit=True)

    def handoff_and_clear(self) -> None:
        since = time.time()
        cmd = self.adapter.command("handoff")
        self.log.info("injecting handoff prompt")
        inject(self.tmux_session, cmd, submit=True)
        try:
            handoff_path = wait_for_handoff(
                self.handoff_dir, since=since, timeout=HANDOFF_WAIT_S,
            )
        except HandoffTimeout:
            self.log.error("handoff timed out — aborting rollover")
            return
        self.log.info("handoff written: %s", handoff_path)
        inject(self.tmux_session, self.adapter.command("clear"), submit=True)
        time.sleep(2)
        resume = (
            f"Read {handoff_path} and continue from where the previous session left off."
        )
        inject(self.tmux_session, resume, submit=True)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="autospec-context-monitor")
    parser.add_argument("--harness", required=True, choices=["claude", "codex", "opencode"])
    parser.add_argument("--tmux-session", required=True)
    parser.add_argument("--cwd", default=os.getcwd())
    parser.add_argument("--handoff-dir", default=".turbo/handoff")
    parser.add_argument("--started-at", type=float, default=time.time())
    parser.add_argument("--log-file", default=None)
    args = parser.parse_args(argv)

    log_path = Path(args.log_file) if args.log_file else (
        Path.home() / ".autospec" / "monitors" / f"{args.tmux_session}.log"
    )
    log_path.parent.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(
        filename=log_path,
        level=logging.INFO,
        format="%(asctime)s %(name)s %(levelname)s %(message)s",
    )
    log = logging.getLogger("context-monitor")

    pid_path = Path.home() / ".autospec" / "monitors" / f"{args.tmux_session}.pid"
    pid_path.parent.mkdir(parents=True, exist_ok=True)
    pid_path.write_text(str(os.getpid()))

    try:
        adapter = load(args.harness)
        hint = {"cwd": args.cwd, "tmux_session": args.tmux_session, "started_at": args.started_at}
        handoff_dir = Path(args.cwd) / args.handoff_dir
        handoff_dir.mkdir(parents=True, exist_ok=True)

        actions = RealActions(args.tmux_session, adapter, handoff_dir, args.started_at)
        engine = Engine(actions)
        log.info("monitor started for %s on tmux=%s", args.harness, args.tmux_session)

        while True:
            if KILL_FLAG.exists():
                log.info("kill flag present — exiting"); return 0
            if not session_exists(args.tmux_session):
                log.info("tmux session gone — exiting"); return 0
            try:
                transcript = adapter.find_transcript(hint)
                usage = adapter.read_usage(transcript)
                log.info("usage: %d/%d (%.1f%%) estimated=%s",
                         usage.used_tokens, usage.max_tokens, usage.pct, usage.estimated)
                engine.tick(usage)
            except InjectionError as e:
                log.error("injection failed: %s", e)
            except Exception as e:
                log.exception("tick failed: %s", e)
            time.sleep(POLL_INTERVAL_S)
    finally:
        pid_path.unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 3: Create `scripts/autospec-context-monitor`**

```bash
#!/usr/bin/env bash
exec python3 -m autospec_context_monitor "$@"
```

Run: `chmod +x scripts/autospec-context-monitor`

- [ ] **Step 4: Run unit test**

Run: `python3 -m pytest tests/context_monitor/test_monitor.py -v`
Expected: PASS.

- [ ] **Step 5: Smoke-test the entrypoint**

Run: `scripts/autospec-context-monitor --help`
Expected: argparse help text printed.

- [ ] **Step 6: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/monitor.py \
        scripts/autospec-context-monitor \
        tests/context_monitor/test_monitor.py
git commit -m "feat(context-monitor): daemon entrypoint with logging + PID file"
```

---

## Task 10: `autospec-session` bash launcher

**Files:**
- Create: `scripts/autospec-session`

- [ ] **Step 1: Write the launcher**

```bash
#!/usr/bin/env bash
# autospec-session: wrap a harness in tmux + spawn context monitor.
# Usage: autospec-session [claude|codex|opencode] [args...]

set -euo pipefail

HARNESS="${1:-}"
if [[ -z "$HARNESS" ]]; then
    echo "usage: autospec-session <claude|codex|opencode> [args...]" >&2
    exit 2
fi
shift

case "$HARNESS" in
    claude|codex|opencode) ;;
    --doctor)
        exec python3 -m autospec_context_monitor.doctor "$@"
        ;;
    --no-monitor)
        # next arg is the real harness
        REAL="$1"; shift
        exec "$REAL" "$@"
        ;;
    *)
        echo "unknown harness: $HARNESS" >&2
        exit 2
        ;;
esac

if ! command -v tmux >/dev/null 2>&1; then
    echo "autospec-session: tmux not found — falling back to direct exec" >&2
    exec "$HARNESS" "$@"
fi

SESSION="as-$(uuidgen | tr 'A-Z' 'a-z' | cut -c1-8)"
CWD="$(pwd)"
STARTED_AT="$(date +%s)"

# Create detached tmux session running the harness.
tmux new-session -d -s "$SESSION" -c "$CWD" "$HARNESS $*"

# Spawn the monitor daemon in the background, detached from the controlling tty.
nohup python3 -m autospec_context_monitor \
    --harness "$HARNESS" \
    --tmux-session "$SESSION" \
    --cwd "$CWD" \
    --started-at "$STARTED_AT" \
    >/dev/null 2>&1 &
MONITOR_PID=$!

# Attach so the user sees the harness; tear monitor down on detach.
trap 'kill "$MONITOR_PID" 2>/dev/null || true' EXIT
tmux attach-session -t "$SESSION"
```

- [ ] **Step 2: Make executable**

Run: `chmod +x scripts/autospec-session`

- [ ] **Step 3: Manual smoke test**

Run: `scripts/autospec-session --doctor` (will fail until Task 11 lands; expected for now — confirm exec path resolves).

Run (skip if tmux interactive testing inconvenient): `scripts/autospec-session claude --version` — should briefly open a tmux session running `claude --version` and exit.

- [ ] **Step 4: Commit**

```bash
git add scripts/autospec-session
git commit -m "feat(context-monitor): autospec-session bash launcher"
```

---

## Task 11: Doctor command

**Files:**
- Create: `packages/autospec_context_monitor/autospec_context_monitor/doctor.py`
- Test: `tests/context_monitor/test_doctor.py`

- [ ] **Step 1: Write failing test**

```python
# tests/context_monitor/test_doctor.py
from autospec_context_monitor.doctor import run_checks


def test_run_checks_returns_dict():
    result = run_checks()
    assert "tmux" in result
    assert "adapters" in result
    assert isinstance(result["adapters"], dict)
    for h in ("claude", "codex", "opencode"):
        assert h in result["adapters"]
```

- [ ] **Step 2: Implement `doctor.py`**

```python
# packages/autospec_context_monitor/autospec_context_monitor/doctor.py
import shutil
import subprocess
import sys
from pathlib import Path

from autospec_context_monitor.adapters import load


def run_checks() -> dict:
    out = {"tmux": None, "harnesses": {}, "adapters": {}, "monitor_pids": []}
    tmux = shutil.which("tmux")
    if tmux:
        v = subprocess.check_output([tmux, "-V"], text=True).strip()
        out["tmux"] = v
    for h in ("claude", "codex", "opencode"):
        out["harnesses"][h] = shutil.which(h) or None
        try:
            load(h)
            out["adapters"][h] = "ok"
        except Exception as e:
            out["adapters"][h] = f"error: {e}"
    pid_dir = Path.home() / ".autospec" / "monitors"
    if pid_dir.exists():
        out["monitor_pids"] = [p.read_text().strip() for p in pid_dir.glob("*.pid")]
    return out


def main(argv=None) -> int:
    result = run_checks()
    print("tmux:           ", result["tmux"] or "NOT FOUND")
    print("harnesses:")
    for h, path in result["harnesses"].items():
        print(f"  {h:10s}{path or 'NOT FOUND'}")
    print("adapters:")
    for h, status in result["adapters"].items():
        print(f"  {h:10s}{status}")
    print("monitor PIDs:   ", result["monitor_pids"] or "(none)")
    bad = (
        result["tmux"] is None
        or any(s != "ok" for s in result["adapters"].values())
    )
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 3: Run test, expect PASS**

Run: `python3 -m pytest tests/context_monitor/test_doctor.py -v`
Expected: 1 passed.

- [ ] **Step 4: Smoke-test**

Run: `python3 -m autospec_context_monitor.doctor`
Expected: printout with tmux version, harness paths (whichever installed), all adapters "ok".

- [ ] **Step 5: Commit**

```bash
git add packages/autospec_context_monitor/autospec_context_monitor/doctor.py \
        tests/context_monitor/test_doctor.py
git commit -m "feat(context-monitor): --doctor diagnostic command"
```

---

## Task 12: `install.sh` integration

**Files:**
- Modify: `install.sh`
- Modify: `uninstall.sh`

- [ ] **Step 1: Read `install.sh` to find the right insertion point**

Run: `Read install.sh` — locate the final block (likely a `print_success` or completion message). Add new section *before* the completion message.

- [ ] **Step 2: Add the opt-in prompt and shell-rc writer**

Insert in `install.sh`:

```bash
# ---- Auto context rollover ---------------------------------------------------

write_auto_rollover_block() {
    local rc="$1"
    [[ -f "$rc" ]] || return 0
    if grep -q "autospec auto-rollover" "$rc"; then
        return 0  # already installed
    fi
    cat >> "$rc" <<'EOF'

# >>> autospec auto-rollover >>>
export AUTOSPEC_AUTO_ROLLOVER=1
if [[ "$AUTOSPEC_AUTO_ROLLOVER" == "1" ]] && command -v autospec-session &>/dev/null; then
    claude()   { autospec-session claude "$@"; }
    codex()    { autospec-session codex "$@"; }
    opencode() { autospec-session opencode "$@"; }
fi
# <<< autospec auto-rollover <<<
EOF
}

prompt_user_for_auto_rollover() {
    [[ "${1:-}" == "--disable-auto-rollover" ]] && return 1
    [[ "${AUTOSPEC_NONINTERACTIVE:-0}" == "1" ]] && return 1
    echo
    echo "Enable autospec auto-context-rollover?"
    echo "  • Compacts at 50% context, writes handoff + clears at 80%"
    echo "  • Requires tmux. Wraps your 'claude'/'codex'/'opencode' commands."
    echo "  • Reversible: re-run install.sh with --disable-auto-rollover"
    read -p "  Enable? [y/N] " ans
    [[ "$ans" == "y" || "$ans" == "Y" ]]
}

if prompt_user_for_auto_rollover "$@"; then
    write_auto_rollover_block "$HOME/.zshrc"
    write_auto_rollover_block "$HOME/.bashrc"
    write_auto_rollover_block "$HOME/.config/fish/config.fish"
    echo "  Auto-rollover enabled. Restart your shell or 'source' your rc."
fi
```

- [ ] **Step 3: Add `--disable-auto-rollover` handling**

In `install.sh` near top:

```bash
if [[ "${1:-}" == "--disable-auto-rollover" ]]; then
    for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.config/fish/config.fish"; do
        [[ -f "$rc" ]] || continue
        sed -i.bak '/# >>> autospec auto-rollover >>>/,/# <<< autospec auto-rollover <<</d' "$rc"
        echo "  Removed auto-rollover block from $rc"
    done
    exit 0
fi
```

- [ ] **Step 4: Mirror removal in `uninstall.sh`**

Add to `uninstall.sh`:

```bash
for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.config/fish/config.fish"; do
    [[ -f "$rc" ]] || continue
    sed -i.bak '/# >>> autospec auto-rollover >>>/,/# <<< autospec auto-rollover <<</d' "$rc"
done
```

- [ ] **Step 5: Symlink scripts into PATH (verify install.sh already does this for scripts/)**

If `install.sh` already symlinks `scripts/*` into `~/.local/bin`, no change needed. Otherwise add:

```bash
ln -sf "$AUTOSPEC_ROOT/scripts/autospec-session" "$HOME/.local/bin/autospec-session"
ln -sf "$AUTOSPEC_ROOT/scripts/autospec-context-monitor" "$HOME/.local/bin/autospec-context-monitor"
```

- [ ] **Step 6: Test the install path non-interactively**

Run: `AUTOSPEC_NONINTERACTIVE=1 bash install.sh` — verify no prompt fires.
Run: `bash install.sh --disable-auto-rollover` — verify it removes any prior block cleanly even if absent.

- [ ] **Step 7: Commit**

```bash
git add install.sh uninstall.sh
git commit -m "feat(context-monitor): install.sh opt-in prompt + shell-rc block"
```

---

## Task 13: `/autospec-rollover-status` skill

**Files:**
- Create: `skills/autospec-rollover-status/SKILL.md`

- [ ] **Step 1: Inspect existing skill format**

Run: `cat skills/autospec-stop/SKILL.md | head -40` — match frontmatter style.

- [ ] **Step 2: Write the skill**

```markdown
---
name: autospec-rollover-status
description: Use when the user wants to check the current autospec auto-rollover monitor status — prints context %, last rollover events, and active monitor PIDs from ~/.autospec/monitors/.
---

# Autospec Rollover Status

Print the current state of the autospec context-rollover monitor for this terminal.

## Steps

1. Check whether `$TMUX` is set. If not, print `not running inside a tmux session — auto-rollover is inactive` and stop.
2. Extract the tmux session name: `tmux display-message -p '#S'`. If it doesn't start with `as-`, print `tmux session not managed by autospec-session` and stop.
3. Read the log at `~/.autospec/monitors/<session>.log`. Print:
   - Most recent `usage:` line (current %).
   - All `injecting` lines from the last hour (rollover events).
   - PID from `~/.autospec/monitors/<session>.pid` if present.
4. If the log doesn't exist, print `no monitor log found for session <session>`.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-rollover-status/SKILL.md
git commit -m "feat(context-monitor): /autospec-rollover-status skill"
```

---

## Task 14: Manual E2E walkthrough doc

E2E tests against real APIs cost real money and shouldn't run in CI by default. Document the manual procedure instead.

**Files:**
- Create: `docs/runbooks/auto-context-rollover-e2e.md`

- [ ] **Step 1: Write the runbook**

```markdown
# Auto Context Rollover — Manual E2E Procedure

For each harness, verify the full rollover cycle end-to-end.

## Prerequisites

- `tmux` 3+ installed
- `install.sh` ran with auto-rollover enabled
- A throwaway shell so the function shim is active

## Per-harness procedure (Claude Code shown; identical for codex/opencode)

1. **Lower threshold for fast verification.** Edit
   `packages/autospec_context_monitor/autospec_context_monitor/engine.py`,
   temporarily set `COMPACT_AT = 1.0` and `ROLLOVER_AT = 2.0`. Reinstall:
   `pip install --user -e packages/autospec_context_monitor`.

2. **Launch.** `claude` (the function routes through `autospec-session`).

3. **Verify monitor spawned.** In a second terminal:
   `cat ~/.autospec/monitors/as-*.pid` — PID should be live.
   `tail -f ~/.autospec/monitors/as-*.log` — should see periodic `usage:` lines.

4. **Trigger compact.** In the harness, send any prompt that produces output.
   Watch the log for `injecting compact:` followed by an actual `/compact`
   appearing in the tmux pane.

5. **Trigger rollover.** Continue prompting. Watch the log for
   `injecting handoff prompt`, then `handoff written: <path>`, then `/clear`
   inject, then the resume prompt inject. Verify the harness UI shows a cleared
   conversation that then reads the handoff and continues.

6. **Restore thresholds.** Revert `engine.py` to 50/80 and reinstall.

## Pass criteria

- Monitor PID file present during session, removed on tmux kill.
- `/compact` and `/clear` arrive in the harness without manual typing.
- Handoff file exists at the path the resume prompt references.
- Resumed session can answer "what were we doing?" coherently.
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/auto-context-rollover-e2e.md
git commit -m "docs(context-monitor): manual E2E runbook"
```

---

## Task 15: README + AGENTS.md docs

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Add README section**

Find an appropriate section in `README.md` (likely after "Skills" or "Install"). Add:

```markdown
### Auto context rollover (opt-in)

`autospec-session` wraps `claude` / `codex` / `opencode` inside tmux and runs a
background monitor. At 50% context the monitor injects `/compact`; at 80% it
asks the session to write a handoff file, then `/clear`s and resumes from the
handoff — all in the same terminal.

Enable during `install.sh`, or later: `install.sh` and answer "y" at the prompt.
Disable: `install.sh --disable-auto-rollover`.

See `docs/specs/2026-05-31-auto-context-rollover-design.md` for design,
`docs/runbooks/auto-context-rollover-e2e.md` for manual verification.
```

- [ ] **Step 2: Add AGENTS.md pointer**

Find the "Skills" or "Tooling" section in `AGENTS.md`. Add:

```markdown
- `autospec-session` wraps harness invocations to enable auto-rollover at 50% / 80% context. Status: `/autospec-rollover-status`.
```

- [ ] **Step 3: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(context-monitor): README + AGENTS.md integration"
```

---

## Final verification

- [ ] **Step 1: Run full test suite**

Run: `python3 -m pytest tests/context_monitor -v`
Expected: all green (skipping tmux tests if tmux unavailable).

- [ ] **Step 2: Run doctor**

Run: `python3 -m autospec_context_monitor.doctor`
Expected: exit code 0, no `error:` lines.

- [ ] **Step 3: Manually walk through the runbook for at least Claude Code**

Follow `docs/runbooks/auto-context-rollover-e2e.md` end-to-end. Note any deviations.

- [ ] **Step 4: Push branch and open PR**

```bash
git push -u origin <branch>
gh pr create --title "feat: auto context rollover for claude/codex/opencode" \
    --body-file docs/specs/2026-05-31-auto-context-rollover-design.md
```

---

## Self-Review Notes

**Spec coverage:**
- §Architecture → Tasks 9, 10
- §Adapter contract → Tasks 2, 3, 4, 5
- §State machine → Task 8
- §Injection layer → Task 6
- §Install integration → Task 12
- §UX (overlays, doctor) → Tasks 11, 12, 13
- §Testing → Tasks 3-8 (Layer 1+2), Task 14 (Layer 3 manual), Task 11 (Layer 4 doctor self-test partially)

**Gap noted:** The spec mentions tmux overlay UX (`tmux display-message`). This is wired into the monitor's logging but NOT into actual overlay calls. Added inline note: extend `RealActions.compact()` and `handoff_and_clear()` to also call `tmux display-message -t <session> "..."` after each injection. Track as a follow-up if not done in Task 9.

**Type consistency:** `Engine(actions)` — `actions` implements the `Actions` Protocol with `compact()` and `handoff_and_clear()`. `RealActions` defines both. `FakeActions` in tests defines both. Consistent.

**Placeholder scan:** No "TBD", "TODO", or "fill in details" survive. The `OPENCODE_MAX_TOKENS = {}` in `models.py` is intentional — opencode adapter falls back to `DEFAULT_MAX_TOKENS` and reads from `models_cache.json` at runtime when a future improvement is made.
