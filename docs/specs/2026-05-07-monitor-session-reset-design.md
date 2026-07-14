# autospec-run Monitor Session Reset (Batch Self-Termination)

**Date:** 2026-05-07  
**Status:** Draft  
**Scope:** `skills/autospec-run/SKILL.md`, `skills/autospec/SKILL.md` (Phase 4), `autospec validate`, `tests/`

---

## 1. Problem

The Phase 4 background monitor runs as a single long-lived background subagent. Foreground subagent transcripts (process(ISSUE) calls) accumulate in the monitor's context window. Empirically this causes context overflow at ~185 tool calls (~4-5 issues), manifesting as either a silent exit or a literal "Prompt is too long" error. The current SKILL.md has no self-termination logic — the monitor either finishes the whole queue or crashes.

## 2. Solution

The monitor self-terminates after processing a configurable batch of issues (default: 3) by writing a status file `~/.autospec/batch-done.json`. The orchestrator's Phase 4 block becomes a relaunch loop: it reads the file after each task-notification and relaunches the monitor with fresh context until the queue is fully drained. All persistent state lives in GitHub labels and heartbeat files, so relaunches are always safe.

---

## 3. Architecture

### 3.1 Monitor outer loop change

Add a `batch_issue_count` counter (initialized to 0) and a `batch_num` parameter (passed in from the orchestrator). After each successful `process(ISSUE)` call, increment `batch_issue_count`. When `batch_issue_count >= AUTOSPEC_BATCH_SIZE` (default 3), write `batch-done.json` and exit.

```
batch_issue_count = 0

while true:
  [existing scan + stop-flag + watchdog logic]
  
  if ready is empty:
    if all issues CLOSED: write ALL_DONE → exit
    else: sleep 300, continue
  
  ISSUE = ready[0]
  claim(ISSUE)
  process(ISSUE)    # foreground subagent
  batch_issue_count += 1
  
  if batch_issue_count >= AUTOSPEC_BATCH_SIZE:
    write BATCH_COMPLETE → exit
  
  # else continue to next iteration immediately
```

The batch counter increments **after each completed `process(ISSUE)` call** (regardless of merge or failure outcome), not on empty scans or blocked-queue sleeps. A fully-blocked queue still reaches the normal 2h idle HARD SHUTDOWN.

The monitor prints the following before writing the file and exiting:
```
[monitor] batch N: processed K/AUTOSPEC_BATCH_SIZE issues — writing batch-done.json and exiting for fresh context
```

### 3.2 Batch-done file

**Path:** `~/.autospec/batch-done.json`

**Schema:**
```json
{
  "batch": 2,
  "processed": 3,
  "repo": "berlinguyinca/autospec",
  "ts": 1234567890,
  "status": "BATCH_COMPLETE"
}
```

`status` is either `BATCH_COMPLETE` (hit batch limit; more issues may remain) or `ALL_DONE` (queue fully drained).

**Lifecycle:**
1. Monitor **deletes** the file at startup (clears stale state from prior crashes).
2. Monitor **writes** it on any intentional exit (batch limit or queue drained).
3. Orchestrator **reads then deletes** it after task-notification, before deciding to relaunch.
4. If the file is **absent** when the orchestrator checks (monitor crashed or overflowed): treat as `BATCH_COMPLETE` and relaunch.

### 3.3 Orchestrator relaunch loop (Phase 4)

Replace the current single-launch Phase 4 with:

```
batch_num = 1

while true:
  launch background monitor (pass batch_num, AUTOSPEC_BATCH_SIZE)
  wait for task-notification

  if ~/.autospec/batch-done.json exists:
    status = read .status from file
    delete ~/.autospec/batch-done.json
  else:
    status = "BATCH_COMPLETE"   # crash/overflow — safe to relaunch

  if status == "ALL_DONE":
    break   # proceed to Phase 6
  else:
    batch_num += 1
    print "[orchestrator] batch N complete — relaunching monitor with fresh context (batch N+1)"
    continue   # immediate relaunch, no sleep
```

### 3.4 Configuration

| Env var | Default | Meaning |
|---|---|---|
| `AUTOSPEC_BATCH_SIZE` | `3` | Max issues per monitor session |

Values ≤ 0 or unset default to 3.

---

## 4. Error handling

| Scenario | Behaviour |
|---|---|
| Monitor crashes / overflows without writing file | Orchestrator finds no file → status = `BATCH_COMPLETE` → relaunch |
| Stale `batch-done.json` from prior session | Monitor deletes at startup → never confuses orchestrator |
| All issues blocked (unmet deps) | Batch counter does not increment on empty scans; normal 2h idle HARD SHUTDOWN fires |
| `AUTOSPEC_BATCH_SIZE` ≥ queue size | Monitor exits with `ALL_DONE` on first batch — no unnecessary relaunch |
| `in-progress-by-bot` left by crashed monitor | Watchdog reconciliation at monitor startup reclaims stale claims normally |

---

## 5. Affected files

| File | Change |
|---|---|
| `skills/autospec-run/SKILL.md` | Add `batch_issue_count` counter to outer loop; add `batch-done.json` write logic; replace single-launch Phase 4 with relaunch loop |
| `skills/autospec/SKILL.md` | Same Phase 4 changes (autospec embeds the full Phase 4 monitor prompt) |
| `skills/autospec-run/codex/prompt.md` | Lock-step sync with SKILL.md body |
| `skills/autospec-run/opencode/agent.md` | Lock-step sync with SKILL.md body |
| `skills/autospec/codex/prompt.md` | Lock-step sync with SKILL.md body |
| `skills/autospec/opencode/agent.md` | Lock-step sync with SKILL.md body |
| `autospec validate` | Add `check_monitor_batch_exit()` function + call in per-skill validation loop |
| `tests/unit/test_monitor_batch_exit.bats` | New bats test file (see §6) |

---

## 6. Testing

### Bats tests (`tests/unit/test_monitor_batch_exit.bats`)

1. **BATCH_COMPLETE signal:** mock monitor writes `batch-done.json` with `status=BATCH_COMPLETE`; verify orchestrator loop re-enters (reads, deletes, increments batch_num).
2. **ALL_DONE signal:** mock monitor writes `status=ALL_DONE`; verify orchestrator breaks and proceeds to Phase 6.
3. **Missing file (crash):** no `batch-done.json` present; verify orchestrator treats as `BATCH_COMPLETE` and relaunches.
4. **Stale file cleanup:** seed a stale `batch-done.json` before monitor start; verify monitor deletes it at startup (file absent after startup hook).
5. **AUTOSPEC_BATCH_SIZE=0 defaults to 3:** verify env-var parsing.

### Smoke test

Run `/autospec-run` against a 4-issue test queue with `AUTOSPEC_BATCH_SIZE=2`. Verify:
- Two monitor launches occur.
- `batch-done.json` is written, read, and deleted between launches.
- All 4 issues close with merged PRs.
- No orphan `in-progress-by-bot` labels or stale heartbeat files.

### Crash recovery test

Kill a monitor mid-batch (no file written). Verify:
- Orchestrator relaunches.
- Watchdog reclaims the `in-progress-by-bot` label.
- The previously claimed issue gets re-processed cleanly.

---

## 7. Out of scope

- ScheduleWakeup-based polling relaunch (Approach C, rejected: event-driven is cleaner).
- Summary-string signal (Approach A, rejected: Approach B is harness-neutral).
- Per-batch partial report aggregation (Phase 6 already collects from GitHub state; no new log needed).
- Changes to `process(ISSUE)` internals — batch logic is purely in the outer loop and orchestrator.
