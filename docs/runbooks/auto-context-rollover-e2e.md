# Auto Context Rollover — Manual E2E Runbook

This runbook documents three manual end-to-end scenarios for exercising the
auto context rollover feature across the supported harnesses: Claude, Codex,
and OpenCode. Each scenario validates the 50% compact threshold and the 80%
handoff+rollover threshold.

**Automation gate:** These scenarios are manual only. CI integration is
deferred. Automate execution only after the `autospec-test` invariant layer is
available.

---

## Prereqs

- `tmux` >= 3.2 installed and on `PATH`.
- At least one harness installed: `claude`, `codex`, or `opencode`.
- The autospec rollover integration installed (`install.sh` run with
  `--enable-auto-rollover`, or opted in via the interactive prompt).
- `AUTOSPEC_AUTO_ROLLOVER=1` exported in your shell (the install block writes
  this to your shell RC; confirm with `echo $AUTOSPEC_AUTO_ROLLOVER`).
- `autospec-session` on `PATH` (verify: `which autospec-session`).
- `bats` installed for the companion test suite
  (`tests/docs/test_runbook.bats`).
- A writable `~/.autospec/` directory with sub-dirs `monitors/` and
  `turbo/handoff/`.

```bash
mkdir -p ~/.autospec/monitors ~/.autospec/turbo/handoff
echo $AUTOSPEC_AUTO_ROLLOVER   # must print 1
which autospec-session          # must resolve
```

---

## Scenario 1 — Claude: 50% compact + 80% handoff rollover

**Purpose:** Verify that a Claude session launched via `autospec-session`
triggers `/compact` at 50% context usage and a full handoff + `/clear` +
resume sequence at 80%.

### Setup

Patch `max_tokens` to a small value so thresholds fire within seconds:

```bash
export AUTOSPEC_CTX_MAX=5000
export AUTOSPEC_AUTO_ROLLOVER=1
SESSION_ID=$(autospec-session claude --dry-run-name-only 2>/dev/null || echo "as-test-claude-$$")
```

### Steps

1. **Launch session:**

   ```bash
   autospec-session claude
   ```

   Expected: A new tmux session named `as-<uuid>` starts and `claude` opens
   inside it. A PID file appears at `~/.autospec/monitors/<tmux_session>.pid`.

   ```bash
   ls ~/.autospec/monitors/   # should list one .pid file
   ```

2. **Seed the context to ~55% (approximately 2 750 tokens):**

   Paste the following prompt into the Claude session:

   ```
   Please write a detailed technical explanation of how context windows work in
   large language models. Cover tokenization, attention mechanisms, sliding
   window approaches, and summarization-based compression. Write at least
   2 500 words with concrete examples.
   ```

3. **Observe the 50% compact event:**

   In a second terminal, tail the monitor log:

   ```bash
   LOG=$(ls ~/.autospec/monitors/*.log 2>/dev/null | head -1)
   tail -f "$LOG"
   ```

   Expected log lines (order matters):

   ```
   event=context_poll pct=5[0-9] harness=claude
   event=compact triggered=true threshold=50 harness=claude
   ```

   Expected UI overlay in the Claude pane:

   ```
   autospec: context at 52% — compacting
   ```

4. **Continue prompting to reach ~85%:**

   Paste additional prompts until context grows past 80%. A short follow-up
   such as:

   ```
   Now extend the explanation with three real-world case studies from
   published LLM systems. Add another 2 000 words minimum.
   ```

5. **Observe the 80% handoff+rollover sequence:**

   Expected log lines:

   ```
   event=context_poll pct=8[0-9] harness=claude
   event=handoff_start harness=claude
   event=handoff_written path=.turbo/handoff/<date>-*.md
   event=clear_sent harness=claude
   event=resume_sent harness=claude
   event=rollover_complete harness=claude
   ```

6. **Verify the handoff file:**

   ```bash
   ls .turbo/handoff/   # must contain a file dated today
   ```

   The file content should include a "## Context" or "## Summary" section
   summarising the prior session.

### Teardown

```bash
MONITOR_PID=$(cat ~/.autospec/monitors/*.pid 2>/dev/null | head -1)
[ -n "$MONITOR_PID" ] && kill -TERM "$MONITOR_PID"
tmux kill-session -t "$(tmux ls 2>/dev/null | grep '^as-' | cut -d: -f1 | head -1)" 2>/dev/null
rm -f ~/.autospec/monitors/*.pid ~/.autospec/monitors/*.log
unset AUTOSPEC_CTX_MAX
```

---

## Scenario 2 — Codex: info-null text-length fallback

**Purpose:** Verify that when the Codex adapter receives `info: null` (no
structured context metadata), it falls back to measuring output text length
and still triggers compact and rollover at the correct thresholds.

### Setup

```bash
export AUTOSPEC_CTX_MAX=5000
export AUTOSPEC_AUTO_ROLLOVER=1
```

Confirm the Codex adapter is active:

```bash
autospec-session codex --doctor 2>&1 | grep -i "adapter.*codex"
```

### Steps

1. **Launch session:**

   ```bash
   autospec-session codex
   ```

   Expected: tmux session `as-<uuid>` starts, `codex` opens inside, PID file
   written to `~/.autospec/monitors/<tmux_session>.pid`.

2. **Force an info-null response:**

   The Codex CLI emits `info: null` when the model response does not include
   structured metadata. To exercise this path, start a session that produces
   a long plaintext response without triggering Codex's native token counter:

   ```
   List the first 500 prime numbers, one per line, with no other text.
   ```

   This response produces dense numeric output that saturates text-length
   counters while Codex reports `info: null`.

3. **Observe the fallback path in the log:**

   ```bash
   LOG=$(ls ~/.autospec/monitors/*.log 2>/dev/null | head -1)
   tail -f "$LOG"
   ```

   Expected log lines:

   ```
   event=context_poll info=null harness=codex fallback=text_length
   event=compact triggered=true threshold=50 harness=codex fallback=text_length
   ```

4. **Continue to 80% and observe rollover:**

   Paste a follow-up prompt that grows output further:

   ```
   Now list the next 500 prime numbers in the same format.
   ```

   Expected log lines:

   ```
   event=context_poll pct=8[0-9] harness=codex fallback=text_length
   event=handoff_start harness=codex
   event=handoff_written path=.turbo/handoff/<date>-*.md
   event=clear_sent harness=codex
   event=resume_sent harness=codex
   event=rollover_complete harness=codex
   ```

5. **Verify handoff file exists:**

   ```bash
   ls .turbo/handoff/   # must list at least one file
   ```

### Teardown

```bash
MONITOR_PID=$(cat ~/.autospec/monitors/*.pid 2>/dev/null | head -1)
[ -n "$MONITOR_PID" ] && kill -TERM "$MONITOR_PID"
tmux kill-session -t "$(tmux ls 2>/dev/null | grep '^as-' | cut -d: -f1 | head -1)" 2>/dev/null
rm -f ~/.autospec/monitors/*.pid ~/.autospec/monitors/*.log
unset AUTOSPEC_CTX_MAX
```

---

## Scenario 3 — OpenCode: SQLite polling

**Purpose:** Verify that the OpenCode adapter reads context usage from the
OpenCode SQLite database (`~/.opencode/opencode.db`) and correctly triggers
compact and rollover at 50% and 80% thresholds.

### Setup

```bash
export AUTOSPEC_CTX_MAX=5000
export AUTOSPEC_AUTO_ROLLOVER=1
```

Confirm OpenCode is installed and the SQLite DB path is accessible:

```bash
autospec-session opencode --doctor 2>&1 | grep -i "sqlite\|opencode.db"
ls ~/.opencode/opencode.db 2>/dev/null || echo "DB not yet created — will appear after first launch"
```

### Steps

1. **Launch session:**

   ```bash
   autospec-session opencode
   ```

   Expected: tmux session `as-<uuid>` starts, `opencode` opens inside, PID
   file written to `~/.autospec/monitors/<tmux_session>.pid`.

2. **Verify SQLite polling is active:**

   In a second terminal:

   ```bash
   LOG=$(ls ~/.autospec/monitors/*.log 2>/dev/null | head -1)
   grep "sqlite\|db_poll" "$LOG" | head -5
   ```

   Expected: lines like `event=db_poll harness=opencode rows=<N>`.

3. **Seed context to ~55%:**

   Paste into the OpenCode session:

   ```
   Explain the architecture of a distributed key-value store like DynamoDB.
   Cover consistent hashing, replication, quorum reads and writes, compaction,
   and failure recovery. Write at least 2 500 words with diagrams described in
   ASCII art.
   ```

4. **Observe the 50% compact event:**

   ```bash
   tail -f "$LOG"
   ```

   Expected log lines:

   ```
   event=db_poll harness=opencode tokens_used=2[0-9]+ tokens_max=5000
   event=compact triggered=true threshold=50 harness=opencode
   ```

5. **Continue to 80% and observe rollover:**

   Follow up with:

   ```
   Extend the explanation to cover global replication, cross-region failover,
   and conflict resolution strategies. Add another 2 000 words.
   ```

   Expected log lines:

   ```
   event=db_poll harness=opencode tokens_used=4[0-9]+ tokens_max=5000
   event=handoff_start harness=opencode
   event=handoff_written path=.turbo/handoff/<date>-*.md
   event=clear_sent harness=opencode
   event=resume_sent harness=opencode
   event=rollover_complete harness=opencode
   ```

6. **Verify handoff file:**

   ```bash
   ls .turbo/handoff/   # must list a file dated today
   ```

   Confirm the SQLite DB was still readable post-rollover (OpenCode keeps the
   DB open across sessions):

   ```bash
   sqlite3 ~/.opencode/opencode.db "SELECT count(*) FROM messages;" 2>/dev/null
   ```

### Teardown

```bash
MONITOR_PID=$(cat ~/.autospec/monitors/*.pid 2>/dev/null | head -1)
[ -n "$MONITOR_PID" ] && kill -TERM "$MONITOR_PID"
tmux kill-session -t "$(tmux ls 2>/dev/null | grep '^as-' | cut -d: -f1 | head -1)" 2>/dev/null
rm -f ~/.autospec/monitors/*.pid ~/.autospec/monitors/*.log
unset AUTOSPEC_CTX_MAX
```

---

## Teardown (global)

If multiple sessions were left open across scenarios:

```bash
# Kill all autospec monitors
for pid_file in ~/.autospec/monitors/*.pid; do
  pid=$(cat "$pid_file" 2>/dev/null)
  [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null
done

# Kill all autospec-managed tmux sessions
tmux ls 2>/dev/null | grep '^as-' | cut -d: -f1 | while read -r s; do
  tmux kill-session -t "$s" 2>/dev/null
done

# Remove monitor state
rm -f ~/.autospec/monitors/*.pid ~/.autospec/monitors/*.log

# Unset patched env vars
unset AUTOSPEC_CTX_MAX
```

---

## Quick reference — expected log line patterns

| Event | Log pattern |
|---|---|
| Context poll (structured) | `event=context_poll pct=<N> harness=<h>` |
| Context poll (text fallback) | `event=context_poll info=null harness=codex fallback=text_length` |
| Context poll (SQLite) | `event=db_poll harness=opencode tokens_used=<N> tokens_max=<M>` |
| Compact triggered at 50% | `event=compact triggered=true threshold=50 harness=<h>` |
| Handoff start at 80% | `event=handoff_start harness=<h>` |
| Handoff written | `event=handoff_written path=.turbo/handoff/<date>-*.md` |
| Clear sent | `event=clear_sent harness=<h>` |
| Resume sent | `event=resume_sent harness=<h>` |
| Rollover complete | `event=rollover_complete harness=<h>` |

---

## Troubleshooting

**Monitor PID file missing after launch:**
Confirm `AUTOSPEC_AUTO_ROLLOVER=1` is set and the rollover shim is active in
your shell RC. Run `autospec-session --doctor` for a diagnostic report.

**Thresholds not firing:**
Verify `AUTOSPEC_CTX_MAX` is set to the patched value. Without the override
the default limit is the harness native value (~200 000 tokens), which requires
much more content to saturate.

**`tmux` sessions not cleaning up:**
Run the global teardown block above. If tmux sessions persist, use
`tmux kill-server` as a last resort (this terminates all tmux sessions on the
machine).

**Handoff file not created:**
Check the monitor log for `event=handoff_error`. Common causes: missing
`.turbo/handoff/` directory, or the harness exited before the handoff command
completed. Re-create the directory with `mkdir -p .turbo/handoff` and retry.
