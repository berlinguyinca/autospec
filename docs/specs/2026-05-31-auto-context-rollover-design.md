# Auto Context Rollover — Design Spec

**Date**: 2026-05-31
**Status**: Draft (awaiting user approval)
**Author**: berlinguyinca
**Scope**: New autospec component that wraps `claude` / `codex` / `opencode` sessions, watches context usage, auto-compacts at 50%, and writes a handoff + clears + resumes at 80% — all within the same terminal session.

---

## Goals

1. Eliminate the manual "context is filling up — should I compact or hand off?" decision during long autospec sessions.
2. Work identically across the three supported harnesses (Claude Code, Codex CLI, OpenCode) — one mental model, per-harness adapters absorb differences.
3. Same-session rollover: at 80%, the user's terminal does not change windows, panes, or processes. The conversation just resets and resumes from a handoff file.
4. Opt-in, reversible, debuggable. No surprises after `install.sh`.

## Non-goals

- Not a context-usage *visualizer* (no fancy TUI, no live %).
- Not a replacement for the harnesses' built-in auto-compact (those still fire at their own thresholds; this layer is additive).
- Not a multi-session orchestrator (one monitor per tmux session, full stop).
- Not a daemon manager (no systemd unit, no launchd — lives and dies with the tmux session).

## Architecture

```
┌─ user terminal ─────────────────────────────────────┐
│  $ claude                  (shell shim/function)    │
│      └─→ autospec-session claude                    │
│              ├─ creates tmux session "as-<uuid>"    │
│              ├─ launches `claude` inside it         │
│              ├─ attaches user terminal to tmux      │
│              └─ spawns monitor daemon (detached)    │
└─────────────────────────────────────────────────────┘
                            │
            ┌───────────────┴──────────────────┐
            │  autospec-context-monitor (PID)  │
            │  loop every 15s:                 │
            │   • adapter.read_usage()         │  ← per-harness
            │   • engine.classify(pct)         │  ← state machine
            │   • injector.send(cmd)           │  ← tmux send-keys
            │   • handoff.wait_for_file()      │  ← reuses /create-handoff
            └──────────────────────────────────┘
```

### Three components

1. **`autospec-session`** (bash launcher, ~150 LOC): picks tmux session name, launches harness inside it, spawns monitor daemon, attaches user. Registers tmux session-end trap to SIGTERM the monitor.
2. **`autospec-context-monitor`** (Python daemon, ~300 LOC core + per-adapter modules): polls, runs the state machine, executes injections.
3. **Adapter modules** (~80 LOC each at `adapters/claude.py`, `adapters/codex.py`, `adapters/opencode.py`): three-method contract isolates harness differences.

## Adapter contract

```python
class HarnessAdapter(Protocol):
    name: str  # "claude" | "codex" | "opencode"

    def find_transcript(self, hint: dict) -> Path:
        """Locate the live transcript for THIS session.
        hint contains: {cwd, tmux_session, pid, started_at}.
        Returns newest matching transcript, or raises TranscriptNotFoundError
        after a 30s wait."""

    def read_usage(self, transcript: Path) -> Usage:
        """Return Usage(used_tokens, max_tokens, model, estimated: bool).
        estimated=True when token counts derived from text length."""

    def command(self, logical: Literal["clear", "compact", "handoff"]) -> str:
        """Map logical command to harness-specific slash command."""
```

### Per-harness implementation matrix

|                       | Claude Code                                              | Codex CLI                                                            | OpenCode                                                                 |
| --------------------- | -------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| **transcript path**   | `~/.claude/projects/<cwd-slug>/<uuid>.jsonl`             | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`                       | `~/.local/share/opencode/opencode.db` (SQLite)                           |
| **usage extraction**  | sum `message.usage.input_tokens + output_tokens`         | sum `event_msg.payload.info.*` when present; fall back to `len/3.5`  | `SELECT SUM(input_tokens + output_tokens) FROM messages WHERE session_id = ?` |
| **max_tokens source** | lookup table by `message.model`                          | lookup table by configured model                                     | `models_cache.json` if present, else lookup table                        |
| **`/clear` mapping**  | `/clear`                                                 | `/new`                                                               | `/new`                                                                   |
| **`/compact` mapping**| `/compact`                                               | `/compact`                                                           | `/compact`                                                               |
| **handoff prompt**    | `Please run /create-handoff and wait for the file to be written before responding further.` | `Write a handoff file to .turbo/handoff/<YYYY-MM-DD>-<slug>.md capturing current task, status, open decisions, in-flight changes, and next step. Confirm the path when done.` | (same as Codex — no slash-command skill system) |

**OpenCode SQLite caveat**: open db with `mode=ro` so live harness writes don't block reads. If schema check at adapter init fails, fall back to `opencode export <sid>` (slower).

**Codex `info: null` caveat**: when explicit token counts are unavailable (version-dependent), the adapter falls back to character-count estimation and marks `estimated=True`. Logged so users know the threshold isn't precise.

## Threshold state machine

```
        ┌─────────┐  pct ≥ 50%   ┌──────────┐  pct ≥ 80%   ┌──────────┐
  ───→  │ NORMAL  │ ───────────→ │ COMPACTED│ ───────────→ │ ROLLED   │
        └─────────┘              └──────────┘              └──────────┘
             ↑                        │                        │
             │  pct drops < 30%       │ pct drops < 30%        │ new transcript
             │ (new transcript after  │ (compaction worked)    │ detected
             │  rollover OR org. low) │                        │ (post /clear)
             └────────────────────────┴────────────────────────┘
```

### Transitions

- **NORMAL → COMPACTED (at 50%)**: inject `command("compact")`. Wait up to 60s for `pct` to drop below 30%. If it doesn't drop, log warning, stay in COMPACTED — don't re-fire.
- **COMPACTED → NORMAL**: if `pct` drops below 30% (compaction worked), reset so the next climb past 50% re-fires.
- **COMPACTED → ROLLED (at 80%)**: three-step sequence:
  1. Inject handoff prompt → wait up to 180s for `.turbo/handoff/<today>-*.md` to appear (mtime poll).
  2. Inject `command("clear")`.
  3. Inject resume prompt: `Read .turbo/handoff/<latest-by-mtime>.md and continue from where the previous session left off.`
- **ROLLED → NORMAL**: after `/clear`/`/new`, the harness writes a new transcript file (same OS process, new session ID). Monitor calls `adapter.find_transcript()` again with `started_at` = rollover timestamp, discovers the new file, and resets to NORMAL. This is what enables a 2nd rollover in a long session.

### Invariants

- A given transition fires at most once per state entry — re-firing requires returning to the prior state first (e.g. NORMAL → COMPACTED → NORMAL → COMPACTED is allowed; back-to-back compacts without an intervening drop are not).
- Polling interval: 15s.
- Injection failure (tmux returns non-zero): retry once after 5s, then abort transition and notify user via terminal bell + `notify-send`/`osascript`.
- Kill switch: `~/.autospec/no-auto-rollover.flag` makes monitor exit on next tick.

## Injection layer

```python
def inject(tmux_session: str, text: str, *, submit: bool = True) -> None:
    subprocess.run(["tmux", "send-keys", "-l", "-t", tmux_session, text],
                   check=True, timeout=5)
    if submit:
        subprocess.run(["tmux", "send-keys", "-t", tmux_session, "Enter"],
                       check=True, timeout=5)
```

`-l` (literal) prevents tmux from interpreting key names like `Up` or `C-c` inside the prompt text. Enter is sent as a separate call so special chars in `text` can't chain commands at the tmux parser layer.

### Pre-injection guards

1. **Prompt-ready check**: `tmux capture-pane -p -t <s> | tail -3` — verify last line ends with the harness's input marker (`>` for CC, `❯` for Codex, `│ >` for OpenCode). If not, wait 2s, retry up to 3 times. Avoids injecting while model is mid-stream.
2. **Session exists**: `tmux has-session -t <s>`. If gone, monitor exits cleanly.
3. **No in-flight injection**: process-local lock (one monitor = one tmux session).

### Logging

Every injection logged to `~/.autospec/monitors/<tmux_session>.log`:
- timestamp
- command sent
- pre-injection pane snapshot (last 3 lines)
- post-injection result (exit code, retry count)

Non-negotiable — only way to debug "why did my session just clear itself".

### Accepted failure modes (no fix)

- User launches harness outside `autospec-session` → no auto-rollover, no error. Opt-in design.
- Nested tmux (tmux-in-tmux) → `send-keys` targets by session name, works; pane capture may be confusing. Documented caveat.
- Detach/re-attach tmux → monitor unaffected.
- Color/ANSI in input marker confuses prompt-detection → fall back to "always inject after 3s quiet period", configurable per-adapter.

## Install integration

`install.sh` gains a new prompt section, opt-in default:

```bash
prompt_user_for_auto_rollover() {
    echo
    echo "Enable autospec auto-context-rollover?"
    echo "  • Compacts at 50% context, writes handoff + clears at 80%"
    echo "  • Requires tmux. Wraps your 'claude'/'codex'/'opencode' commands."
    echo "  • Reversible: re-run install.sh with --disable-auto-rollover"
    read -p "  Enable? [y/N] " ans
    [[ "$ans" == "y" || "$ans" == "Y" ]]
}
```

On yes: writes a sourced block to `~/.zshrc`, `~/.bashrc`, and/or `~/.config/fish/config.fish` (whichever exist):

```bash
# >>> autospec auto-rollover >>>
export AUTOSPEC_AUTO_ROLLOVER=1
if [[ "$AUTOSPEC_AUTO_ROLLOVER" == "1" ]] && command -v autospec-session &>/dev/null; then
    claude()   { autospec-session claude "$@"; }
    codex()    { autospec-session codex "$@"; }
    opencode() { autospec-session opencode "$@"; }
fi
# <<< autospec auto-rollover <<<
```

Functions (not aliases) so they expand in non-interactive shells. `command claude` bypasses the function when needed.

### Escape hatches

- `AUTOSPEC_AUTO_ROLLOVER=0 claude` — one-shot disable.
- `command claude` — bypass function entirely.
- `touch ~/.autospec/no-auto-rollover.flag` — global kill switch; running monitors exit on next tick.
- `autospec-session --no-monitor claude` — launch in tmux but skip monitor.

### Uninstall

- `install.sh --disable-auto-rollover` removes the sourced block and unsets the env var.
- `uninstall.sh` removes both the block and the binaries.

## User-facing UX

- **At 50% trigger**: terminal bell + `tmux display-message` overlay: `"autospec: context at 52% — compacting"`. 3s.
- **At 80% trigger**: overlay `"autospec: context at 81% — writing handoff + rolling session"`. 5s.
- **On rollover complete**: overlay `"autospec: new session loaded with handoff at <path>"`.
- **On failure**: overlay stays until dismissed + terminal bell + logged.

### Doctor command

`autospec-session --doctor` prints:
- tmux version
- harness binaries detected
- transcript paths writable
- adapter import success
- current monitor PIDs
- recent rollover events from logs

### Discoverability

- README section added.
- AGENTS.md one-line pointer added.
- New skill `/autospec-rollover-status` (~30 LOC): reads the live monitor log, tells the running Claude session "you're at X% context, last rollover at Y" — so an agent can self-check.

## Testing strategy

### Layer 1 — Adapter unit tests (fast, hermetic)

Fixtures under `tests/fixtures/<harness>/`:
- `claude/short-session.jsonl`, `claude/sonnet-1m.jsonl`, `claude/missing-usage.jsonl`
- `codex/info-populated.jsonl`, `codex/info-null.jsonl`
- `opencode/session.db`, `opencode/schema-changed.db`

Assert: `find_transcript` picks newest matching file; `read_usage` returns correct tuple including `estimated` flag; `command()` returns correct strings.

### Layer 2 — State-machine tests (mocked adapter)

Drive engine with scripted `Usage` sequences (e.g. `[10%, 30%, 51%, 25%, 49%, 52%, 75%, 81%]`). Assert exact injection sequence. Covers anti-flapping invariants, COMPACTED→NORMAL reset, "stuck compacted" path.

### Layer 3 — End-to-end with real harnesses (slow, gated)

Under `tests/e2e/`, runs only when `AUTOSPEC_E2E=1`. Per-harness scenario:
1. Spawn real tmux session.
2. Launch harness with scripted prompt that forces predictable context growth.
3. Patch `max_tokens` to small value (e.g. 5000) so 50% fires in seconds.
4. Assert: `/compact` injected, transcript shrinks, `/create-handoff` ran, handoff file exists, `/clear` fired, resume prompt sent, new transcript appeared.
5. Tear down.

Codex E2E variant exercises the `info: null` text-length fallback. OpenCode E2E variant exercises SQLite polling.

CI nightly, not per-PR. Per-PR runs Layers 1–2 only.

### Layer 4 — Doctor self-test

`autospec-session --doctor --self-test` runs Layers 1+2 against installed adapters; exits non-zero on failure. Used by `install.sh` to verify a working install.

### Anti-flake measures

- `tmux send-keys` in tests goes through a `--dry-run` recorder.
- Time-based waits (60s post-compact, 180s handoff) monkeypatched to 0.5s.
- Fixtures checked in, not generated at test time — recorded once from real sessions, PII-redacted.

### Out of scope

- Cross-harness combinatorial coverage.
- tmux itself.
- Harness network behavior.

## File layout

```
autospec/
├── scripts/
│   ├── autospec-session                  # bash launcher
│   └── autospec-context-monitor          # python daemon entrypoint
├── packages/
│   └── autospec_context_monitor/         # python pkg
│       ├── __init__.py
│       ├── engine.py                     # state machine
│       ├── injector.py                   # tmux send-keys wrapper
│       ├── handoff.py                    # handoff-file polling
│       ├── doctor.py                     # --doctor implementation
│       └── adapters/
│           ├── __init__.py
│           ├── base.py                   # Protocol + Usage dataclass
│           ├── claude.py
│           ├── codex.py
│           └── opencode.py
├── skills/
│   └── autospec-rollover-status/         # new ~30 LOC skill
├── tests/
│   ├── adapters/
│   ├── engine/
│   ├── e2e/
│   └── fixtures/
└── install.sh                            # gains prompt_user_for_auto_rollover()
```

## Open questions

None blocking. Items to revisit during implementation:

- Whether the prompt-detection heuristic per harness needs tuning (TBD after first manual e2e session).
- Whether `models_cache.json` is reliably present for OpenCode or always needs the hardcoded fallback.
- Whether the `/autospec-rollover-status` skill should be auto-invoked by some other autospec skill, or only manually.

## Acceptance criteria

- A user with `AUTOSPEC_AUTO_ROLLOVER=1` running `claude` (or `codex` or `opencode`) gets:
  - Their session launched inside a tmux session named `as-<uuid>`.
  - A monitor PID file at `~/.autospec/monitors/<tmux_session>.pid`.
  - Auto-`/compact` when context crosses 50%, observed in the conversation UI.
  - Auto-handoff + `/clear` + resume when context crosses 80%, with the new session reading the handoff and continuing the prior task.
- All Layer 1 + Layer 2 tests pass in CI.
- `autospec-session --doctor` exits 0 on a fresh install.
- `install.sh --disable-auto-rollover` cleanly removes the integration.
