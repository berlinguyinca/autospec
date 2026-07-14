# Monitor Session Reset (Batch Self-Termination) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Phase 4 autospec-run monitor self-terminate after processing `AUTOSPEC_BATCH_SIZE` issues (default 3), write `~/.autospec/batch-done.json`, and let the orchestrator relaunch it with fresh context.

**Architecture:** The monitor's outer loop gains a `batch_issue_count` counter that increments after every `process(ISSUE)` call (merge or failure). At the batch limit it writes `batch-done.json` with `status=BATCH_COMPLETE` and exits. The orchestrator's Phase 4 launch block becomes a `while` loop that relaunches the monitor until it sees `ALL_DONE` (or the file is absent, treating a crash as `BATCH_COMPLETE`). All persistent state remains in GitHub labels and heartbeat files — relaunches are always safe.

**Tech Stack:** Markdown (SKILL.md prompt files), Bash (validate.sh, bats tests), JSON (batch-done.json), lock-step trio sync (SKILL.md = codex/prompt.md = opencode/agent.md body).

---

## File Map

| Action | Path | What changes |
|---|---|---|
| Modify | `autospec validate` | Add `check_monitor_batch_exit()` function + call it in main loop |
| Create | `tests/unit/test_monitor_batch_exit.bats` | 5 bats tests covering batch-done.json protocol |
| Modify | `skills/autospec-run/SKILL.md` | Orchestrator relaunch loop + monitor outer loop batch logic |
| Modify | `skills/autospec-run/codex/prompt.md` | Lock-step sync (body = SKILL.md body stripped of frontmatter) |
| Modify | `skills/autospec-run/opencode/agent.md` | Lock-step sync (frontmatter preserved, body replaced) |
| Modify | `skills/autospec/SKILL.md` | Same Phase 4 changes as autospec-run |
| Modify | `skills/autospec/codex/prompt.md` | Lock-step sync |
| Modify | `skills/autospec/opencode/agent.md` | Lock-step sync |

---

## Task 1: Add `check_monitor_batch_exit()` to validate.sh

**Files:**
- Modify: `autospec validate`

- [ ] **Step 1: Find the insertion point in validate.sh**

Run:
```bash
grep -n "^check_harness_detection_block\|^check_subagent_model_tier\|^main(" autospec validate | head -10
```
Expected: line numbers for existing check functions and the main call loop.

- [ ] **Step 2: Add the function after `check_harness_detection_block`**

Insert this function in `autospec validate` immediately after the closing `}` of `check_harness_detection_block`:

```bash
check_monitor_batch_exit() {
    local file="$1"
    # Only enforce on skills that contain a Phase 4 monitor outer loop.
    # Detect by presence of "batch_issue_count" or "AUTOSPEC_BATCH_SIZE".
    # Skills without Phase 4 (e.g. autospec-classify) are silently skipped.
    if ! grep -q "Phase 4" "$file" 2>/dev/null; then
        return 0
    fi
    local missing=()
    grep -q "batch_issue_count" "$file"    || missing+=("batch_issue_count")
    grep -q "AUTOSPEC_BATCH_SIZE"  "$file" || missing+=("AUTOSPEC_BATCH_SIZE")
    grep -q "batch-done.json"      "$file" || missing+=("batch-done.json")
    grep -q "BATCH_COMPLETE"       "$file" || missing+=("BATCH_COMPLETE")
    grep -q "ALL_DONE"             "$file" || missing+=("ALL_DONE")
    if [ "${#missing[@]}" -gt 0 ]; then
        fail "$file: monitor batch-exit missing: ${missing[*]}"
    fi
    info "monitor-batch-exit: $(basename "$(dirname "$file")")"
}
```

- [ ] **Step 3: Call it from the per-skill validation loop**

Find the line in validate.sh that calls `check_harness_detection_block "$skill_dir/SKILL.md"` (or similar per-file check loop). Add the new call immediately after:

```bash
check_monitor_batch_exit "$skill_dir/SKILL.md"
```

- [ ] **Step 4: Verify validate.sh is parseable**

```bash
bash -n autospec validate
```
Expected: no output (no syntax errors).

- [ ] **Step 5: Commit**

```bash
git add autospec validate
git commit -m "feat(validate): add check_monitor_batch_exit() for batch self-termination"
```

---

## Task 2: Write failing bats tests (TDD red phase)

**Files:**
- Create: `tests/unit/test_monitor_batch_exit.bats`

- [ ] **Step 1: Create the test file**

```bash
cat > tests/unit/test_monitor_batch_exit.bats << 'BATS'
#!/usr/bin/env bats
# tests/unit/test_monitor_batch_exit.bats — verify check_monitor_batch_exit()
# in autospec validate detects missing/present batch self-termination logic.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/autospec validate"
    SCRATCH="$(mktemp -d)"
    export SCRATCH REPO_ROOT VALIDATE

    # Build an isolated helper that exposes only check_monitor_batch_exit.
    HELPER="$SCRATCH/helper.sh"
    cat > "$HELPER" <<'HELPER_SCRIPT'
#!/usr/bin/env bash
set -eu
fail() { printf 'validate: FAIL — %s\n' "$*" >&2; exit 1; }
info() { printf 'validate: %s\n' "$*"; }
HELPER_SCRIPT
    sed -n '/^check_monitor_batch_exit()/,/^}/p' "$VALIDATE" >> "$HELPER"
    chmod +x "$HELPER"
    export HELPER
}

teardown() {
    rm -rf "$SCRATCH"
}

# Helper: write a minimal SKILL.md body with Phase 4 marker and all required tokens.
_full_batch_skill() {
    cat <<'MD'
## Phase 4 — Background autonomous monitor

> batch_issue_count=0
> AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-3}
> batch-done.json
> BATCH_COMPLETE
> ALL_DONE
MD
}

# Helper: write a Phase 4 skill missing one token.
_missing_token_skill() {
    local missing="$1"
    _full_batch_skill | grep -v "$missing"
}

@test "SKILL.md with all batch-exit tokens passes" {
    local f="$SCRATCH/SKILL.md"
    _full_batch_skill > "$f"
    run bash "$HELPER" check_monitor_batch_exit "$f"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "monitor-batch-exit"
}

@test "SKILL.md missing batch_issue_count fails" {
    local f="$SCRATCH/SKILL.md"
    _missing_token_skill "batch_issue_count" > "$f"
    run bash "$HELPER" check_monitor_batch_exit "$f"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "batch_issue_count"
}

@test "SKILL.md missing batch-done.json fails" {
    local f="$SCRATCH/SKILL.md"
    _missing_token_skill "batch-done.json" > "$f"
    run bash "$HELPER" check_monitor_batch_exit "$f"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "batch-done.json"
}

@test "SKILL.md without Phase 4 marker is silently skipped" {
    local f="$SCRATCH/SKILL.md"
    echo "# autospec-classify — no Phase 4 here" > "$f"
    run bash "$HELPER" check_monitor_batch_exit "$f"
    [ "$status" -eq 0 ]
}

@test "SKILL.md missing ALL_DONE fails" {
    local f="$SCRATCH/SKILL.md"
    _missing_token_skill "ALL_DONE" > "$f"
    run bash "$HELPER" check_monitor_batch_exit "$f"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "ALL_DONE"
}
BATS
```

- [ ] **Step 2: Run the tests — expect failures (red phase)**

```bash
bats tests/unit/test_monitor_batch_exit.bats
```
Expected: tests 1, 2, 3, 5 fail (function doesn't exist yet in helper); test 4 may pass. This confirms TDD setup is correct.

- [ ] **Step 3: Commit the failing tests**

```bash
git add tests/unit/test_monitor_batch_exit.bats
git commit -m "test(monitor): add failing bats for batch self-termination (TDD red)"
```

---

## Task 3: Update autospec-run/SKILL.md — orchestrator relaunch loop

**Files:**
- Modify: `skills/autospec-run/SKILL.md`

- [ ] **Step 1: Replace the single-launch sentence with the relaunch loop**

Find this text (around line 204):
```
Then launch a **background subagent** with this prompt verbatim:
```

Replace it with:
```
Then launch a **background monitor loop** — the orchestrator relaunches the monitor with fresh context after each batch of `AUTOSPEC_BATCH_SIZE` issues (default: 3). The monitor is stateless: all persistent state lives in GitHub labels and heartbeat files, so relaunches are always safe.

```
batch_num = 1
while true:
  launch background subagent (pass batch_num; set AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-3})
  wait for task-notification (monitor agent completes)

  # Read and consume the batch-done signal.
  if [ -f "$HOME/.autospec/batch-done.json" ]; then
    status=$(jq -r .status "$HOME/.autospec/batch-done.json" 2>/dev/null || echo "BATCH_COMPLETE")
    rm -f "$HOME/.autospec/batch-done.json"
  else
    status="BATCH_COMPLETE"   # monitor crashed / overflowed — safe to relaunch
  fi

  if [ "$status" = "ALL_DONE" ]; then
    break   # proceed to Phase 6 final report
  fi

  batch_num=$((batch_num + 1))
  echo "[orchestrator] batch $((batch_num - 1)) complete — relaunching monitor with fresh context (batch ${batch_num})"
  # continue immediately, no sleep
```

Pass the following prompt verbatim to each background subagent:
```

- [ ] **Step 2: Update the "Never exit after one issue" warning**

Find:
```
> **Never exit after processing one issue** — the loop must persist until shutdown (idle timeout, stop.flag, or all issues resolved).
```

Replace with:
```
> **Session batching:** Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default 3) by writing `~/.autospec/batch-done.json` with `status=BATCH_COMPLETE`. The orchestrator will relaunch you with fresh context. When the queue is fully drained, write `status=ALL_DONE` instead. This keeps each monitor session short to prevent context overflow.
```

- [ ] **Step 3: Add batch initialization at the top of the outer loop**

Find the outer loop start (around line 229):
```
> while true:
>   deferred = []   # issues skipped because they exceed the active profile
```

Replace with:
```
> while true:
>   deferred = []   # issues skipped because they exceed the active profile
>
>   # Batch self-termination setup (run-once, at first iteration only via shell init before loop).
>   # These vars are initialized before the loop:
>   #   batch_issue_count=0
>   #   BATCH_SIZE="${AUTOSPEC_BATCH_SIZE:-3}"
>   #   rm -f "$HOME/.autospec/batch-done.json"   # clear any stale file from prior crash
```

- [ ] **Step 4: Add batch counter increment and exit after process(ISSUE)**

Find (around line 370-372):
```
>   process(ISSUE)   # foreground subagent — see template below
>   # Immediate next-issue pickup: NO SLEEP after process(ISSUE). Re-enter the top
>   # of this loop immediately so the fresh queue scan can pick any issue unblocked
```

Replace with:
```
>   process(ISSUE)   # foreground subagent — see template below
>   batch_issue_count=$((batch_issue_count + 1))
>   if [ "$batch_issue_count" -ge "$BATCH_SIZE" ]; then
>     printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"BATCH_COMPLETE"}\n' \
>       "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>       > "$HOME/.autospec/batch-done.json"
>     echo "[monitor] batch ${batch_num:-1}: processed $batch_issue_count/$BATCH_SIZE issues — writing batch-done.json and exiting for fresh context"
>     exit 0
>   fi
>   # Immediate next-issue pickup: NO SLEEP after process(ISSUE). Re-enter the top
>   # of this loop immediately so the fresh queue scan can pick any issue unblocked
```

- [ ] **Step 5: Add ALL_DONE write to the HARD SHUTDOWN path**

Find (around line 283):
```
>     if open_count == 0 AND latest_close > 2h ago: HARD SHUTDOWN — emit final report (incl. deferred summary, see Phase 6)
```

Replace with:
```
>     if open_count == 0 AND latest_close > 2h ago:
>       printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
>         "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>         > "$HOME/.autospec/batch-done.json"
>       echo "[monitor] all issues processed — writing ALL_DONE and exiting"
>       HARD SHUTDOWN — emit final report (incl. deferred summary, see Phase 6)
```

- [ ] **Step 6: Verify validate.sh now passes on autospec-run SKILL.md**

```bash
autospec validate 2>&1 | grep -E "FAIL|PASS|monitor-batch"
```
Expected: `monitor-batch-exit: autospec-run` line (PASS); no FAIL for autospec-run.

- [ ] **Step 7: Commit**

```bash
git add skills/autospec-run/SKILL.md
git commit -m "feat(autospec-run): add batch self-termination + orchestrator relaunch loop"
```

---

## Task 4: Lock-step sync autospec-run trio

**Files:**
- Modify: `skills/autospec-run/codex/prompt.md`
- Modify: `skills/autospec-run/opencode/agent.md`

- [ ] **Step 1: Strip frontmatter from SKILL.md to get raw body**

```bash
awk 'BEGIN{f=0} /^---/{f++; next} f>=2{print}' \
  skills/autospec-run/SKILL.md > /tmp/autospec-run-body.md
wc -l /tmp/autospec-run-body.md
```
Expected: line count matches SKILL.md minus its frontmatter lines.

- [ ] **Step 2: Overwrite codex/prompt.md**

```bash
cp /tmp/autospec-run-body.md skills/autospec-run/codex/prompt.md
```

- [ ] **Step 3: Update opencode/agent.md (keep frontmatter, replace body)**

```bash
# Extract frontmatter (up to and including the second ---):
awk 'BEGIN{f=0} /^---/{f++; print; if(f==2) exit} f>0{print}' \
  skills/autospec-run/opencode/agent.md > /tmp/ocode-front.md
# Combine: frontmatter + blank line + body:
{ cat /tmp/ocode-front.md; echo ""; cat /tmp/autospec-run-body.md; } \
  > skills/autospec-run/opencode/agent.md
```

- [ ] **Step 4: Run validate.sh to confirm lock-step passes**

```bash
autospec validate 2>&1 | grep -E "lock-step|FAIL"
```
Expected: no FAIL; `lock-step: autospec-run` printed.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/codex/prompt.md skills/autospec-run/opencode/agent.md
git commit -m "chore(autospec-run): lock-step sync codex/prompt.md and opencode/agent.md"
```

---

## Task 5: Apply same Phase 4 changes to autospec/SKILL.md + lock-step sync

**Files:**
- Modify: `skills/autospec/SKILL.md`
- Modify: `skills/autospec/codex/prompt.md`
- Modify: `skills/autospec/opencode/agent.md`

- [ ] **Step 1: Apply identical Phase 4 text changes to autospec/SKILL.md**

Repeat every edit from Task 3 Steps 1–5, targeting `skills/autospec/SKILL.md`. The Phase 4 section in autospec/SKILL.md is a verbatim copy of the one in autospec-run/SKILL.md (they share the same monitor prompt text). Use grep to find the equivalent line numbers:

```bash
grep -n "Then launch a \*\*background subagent\|Never exit after processing\|while true:\|HARD SHUTDOWN\|process(ISSUE)   # foreground" \
  skills/autospec/SKILL.md
```

Apply the same five substitutions from Task 3 at the lines found above.

- [ ] **Step 2: Verify validate.sh passes for autospec**

```bash
autospec validate 2>&1 | grep -E "FAIL|monitor-batch"
```
Expected: `monitor-batch-exit: autospec` printed; no FAIL.

- [ ] **Step 3: Commit autospec/SKILL.md**

```bash
git add skills/autospec/SKILL.md
git commit -m "feat(autospec): add batch self-termination + orchestrator relaunch loop"
```

- [ ] **Step 4: Strip frontmatter and sync autospec codex/opencode trios**

```bash
awk 'BEGIN{f=0} /^---/{f++; next} f>=2{print}' \
  skills/autospec/SKILL.md > /tmp/autospec-body.md

cp /tmp/autospec-body.md skills/autospec/codex/prompt.md

awk 'BEGIN{f=0} /^---/{f++; print; if(f==2) exit} f>0{print}' \
  skills/autospec/opencode/agent.md > /tmp/ocode-autospec-front.md
{ cat /tmp/ocode-autospec-front.md; echo ""; cat /tmp/autospec-body.md; } \
  > skills/autospec/opencode/agent.md
```

- [ ] **Step 5: Validate lock-step for autospec**

```bash
autospec validate 2>&1 | grep -E "lock-step|FAIL"
```
Expected: `lock-step: autospec` printed; no FAIL.

- [ ] **Step 6: Commit**

```bash
git add skills/autospec/codex/prompt.md skills/autospec/opencode/agent.md
git commit -m "chore(autospec): lock-step sync codex/prompt.md and opencode/agent.md"
```

---

## Task 6: Green phase — run all tests

**Files:** none new

- [ ] **Step 1: Run the bats test suite**

```bash
bats tests/unit/test_monitor_batch_exit.bats -v
```
Expected: all 5 tests PASS.

- [ ] **Step 2: Run the full validate.sh**

```bash
autospec validate
```
Expected: exit 0; no FAIL lines; `monitor-batch-exit: autospec` and `monitor-batch-exit: autospec-run` both printed.

- [ ] **Step 3: Run full bats suite to catch regressions**

```bash
bats tests/unit/ --timing
```
Expected: all tests pass; `test_monitor_batch_exit.bats` shows 5 passing.

- [ ] **Step 4: Final commit if anything was fixed**

```bash
git add -p   # review any fixup changes
git commit -m "fix(monitor): green phase — all batch-exit tests passing"
```
Skip if nothing changed since last commit.

---

## Self-Review Notes

- **Spec coverage:** §3.1 (batch counter) → Task 3 Steps 3–4. §3.2 (file schema) → Task 3 Step 4. §3.3 (orchestrator loop) → Task 3 Step 1. §3.4 (config) → Task 3 Steps 3–4. §4 (error handling) → Task 3 Step 5 (crash recovery) + Task 3 Step 3 (stale file). §5 (affected files) → Tasks 3–5. §6 (bats tests) → Task 2.
- **Lock-step:** Explicitly handled in Tasks 4 and 5 for all 6 trio files.
- **TDD:** Task 2 writes and commits failing tests before Task 3 writes the implementation.
- **Type consistency:** `batch_issue_count`, `AUTOSPEC_BATCH_SIZE`, `batch-done.json`, `BATCH_COMPLETE`, `ALL_DONE` — used consistently across validate.sh function (Task 1), bats tests (Task 2), and SKILL.md prose (Tasks 3, 5).
