# End-of-Run Gap Remediation Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After an autospec run drains its `auto-implement` queue, automatically broad-review the shipped work, filter false positives, file every surviving gap as an `auto-implement` issue, and re-run the implementation loop to close them — bounded by a configurable round cap.

**Architecture:** Extend `/autospec-review` with a `--remediation` + `--emit-gaps <path>` mode (Approach A from the spec) that emits a machine-readable gap JSON. A new deterministic `gap-remediation-loop.sh` reads that JSON, dedupes against open issues + `docs:drift` self-heal, and files survivors. `/autospec-run` gains a bounded **Phase 5.5** (replacing the report-only post-batch audit) that drives review → file → monitor-drain → re-review until convergence or `AUTOSPEC_GAP_MAX_ROUNDS` (default 2). All SKILL.md edits propagate across the lock-step trio (SKILL.md + codex/prompt.md + opencode/agent.md) for both the `autospec-run` and `autospec-review` skills, with `autospec validate` gaining named-section checks.

**Tech Stack:** Bash 3.2+ (shared scripts), `gh` CLI (GitHub), `jq` (JSON), `bats` 1.13 (unit tests), markdown SKILL/prompt/agent trio files, `autospec validate` lock-step harness.

---

## Lock-step trio rule (READ BEFORE ANY SKILL.md EDIT)

`autospec validate` `check_lockstep()` (validate.sh:55-67) byte-diffs the **body** of every `skills/<skill>/SKILL.md` against its `codex/prompt.md` and `opencode/agent.md` siblings. The "body" is computed by `strip_body()` = `awk '/^---$/{c++; next} c>=2'` (everything AFTER the second `---` line).

Therefore **every prose/markdown edit to a SKILL.md body MUST be applied byte-identically to all three trio files.** The two skills touched here have DIFFERENT head conventions — verify the exact bytes already present before editing:

- **`skills/autospec-run/`**: `SKILL.md` opens with YAML frontmatter (`---\nname: ...\n---`). `codex/prompt.md` opens with ONE leading blank line then `# autospec-run workflow (harness-neutral)` (NO `<!-- BODY START -->` marker for this skill). `opencode/agent.md` opens with `---\ndescription: ...\nmode: primary\n---` then the body.
- **`skills/autospec-review/`**: `SKILL.md` has YAML frontmatter. `codex/prompt.md` opens with ONE leading blank line then `<!-- BODY START -->` then the body. `opencode/agent.md` opens with `---\ndescription / mode: primary\n---` then `<!-- BODY START -->` then the body.

After editing any trio, the canonical verification is `autospec validate` (it will print `lock-step: <skill>` and fail with a `diff` if bodies diverge). Run it after every trio-touching task.

## File Structure

- **Create** `skills/autospec-shared/scripts/gap-remediation-loop.sh` — deterministic gap-filing driver (reads gap JSON, dedupes, files `gh` issues, tracks round state, enforces round cap).
- **Create** `skills/autospec-shared/tests/unit/gap-remediation-loop.bats` — unit tests for the driver.
- **Create** `skills/autospec-shared/tests/unit/run-batch-start.bats` — unit tests for the batch-start reader.
- **Create** `skills/autospec-shared/tests/unit/autospec-review-remediation.bats` — schema/filter tests for the review remediation emitter helper.
- **Create** `skills/autospec-shared/scripts/gap-json-lib.sh` — small sourceable helper: validates one gap object's schema + computes a title-hash dedupe fallback. Used by both the loop driver and the review-mode bats so the JSON contract lives in exactly one place (DRY).
- **Create** `skills/autospec-shared/scripts/run-batch-start.sh` — tiny write/read helper for `~/.autospec/.run-batch-start` (UTC ISO run-start timestamp).
- **Modify** `skills/autospec-run/SKILL.md` + `codex/prompt.md` + `opencode/agent.md` — add BATCH_START capture to the run-start section; replace the `## Post-batch audit` section with `## Phase 5.5 — End-of-run gap remediation`.
- **Modify** `skills/autospec-review/SKILL.md` + `codex/prompt.md` + `opencode/agent.md` — add `--remediation` + `--emit-gaps` flags, broad-dimension review prose, false-positive filter step, gap-JSON emission contract.
- **Modify** `autospec validate` — clone `check_stop_mode_section()` into two new named-section checks; register at the call site.

---

## Task 1: BATCH_START capture helper + run-start wiring

The spec's Phase 5.5 reads `BATCH_START_DATE`, but `skills/autospec-run/SKILL.md:844` references it without ever assigning it. This task formalizes a run-start write of `~/.autospec/.run-batch-start` (UTC ISO 8601) and a reader, then wires the write into the autospec-run run-start section across the trio.

**Files:**
- Create: `skills/autospec-shared/scripts/run-batch-start.sh`
- Create: `skills/autospec-shared/tests/unit/run-batch-start.bats`
- Modify: `skills/autospec-run/SKILL.md` (run-start section ~:200) + `codex/prompt.md` + `opencode/agent.md`

- [ ] **Step 1: Write the failing test**

Create `skills/autospec-shared/tests/unit/run-batch-start.bats`:

```bash
#!/usr/bin/env bats
# run-batch-start.bats — tests for run-batch-start.sh (write/read of ~/.autospec/.run-batch-start)

SCRIPT="${BATS_TEST_DIRNAME}/../../../skills/autospec-shared/scripts/run-batch-start.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "run-batch-start.sh is executable" {
    [ -x "$SCRIPT" ]
}

@test "run-batch-start.sh --help exits 0 and prints Usage" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

@test "--write creates the batch-start file with a UTC ISO timestamp" {
    run bash "$SCRIPT" --write
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/.run-batch-start" ]
    # ISO 8601 UTC, e.g. 2026-05-24T18:42:11Z
    run cat "$TEST_TMP/.run-batch-start"
    [[ "$output" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]
}

@test "--read echoes the previously written timestamp" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --read
    [ "$status" -eq 0 ]
    [ "$output" = "2026-05-24T00:00:00Z" ]
}

@test "--read with no file falls back to epoch sentinel and exits 0" {
    run bash "$SCRIPT" --read
    [ "$status" -eq 0 ]
    [ "$output" = "1970-01-01T00:00:00Z" ]
}

@test "--write does not overwrite an existing batch-start (idempotent within a run)" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --write
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/.run-batch-start"
    [ "$output" = "2026-05-24T00:00:00Z" ]
}

@test "--write --force overwrites an existing batch-start" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --write --force
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/.run-batch-start"
    [ "$output" != "2026-05-24T00:00:00Z" ]
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/run-batch-start.bats`
Expected: FAIL — every test errors with "run-batch-start.sh: No such file or directory" / not executable.

- [ ] **Step 3: Write minimal implementation**

Create `skills/autospec-shared/scripts/run-batch-start.sh`:

```bash
#!/usr/bin/env bash
# run-batch-start.sh — record / read the UTC run-start timestamp for an autospec run.
#
# autospec-run writes this once at run-start so the end-of-run gap-remediation
# phase can scope `/autospec-review --since` to work shipped during THIS run.
# Format: ISO 8601 UTC, e.g. 2026-05-24T18:42:11Z (matches `date -u +%Y-%m-%dT%H:%M:%SZ`).
#
# Usage:
#   run-batch-start.sh --write [--force]   # write now() unless file exists (--force overwrites)
#   run-batch-start.sh --read              # echo stored timestamp (epoch sentinel if absent)
#   run-batch-start.sh --help
#
# Environment:
#   AUTOSPEC_STATE_DIR  — override state directory (default: ~/.autospec)
#
# Exit codes:
#   0  always (best-effort; missing file on --read returns epoch sentinel)
#
# Requires: bash 3.2+, date

set +e

STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
BATCH_FILE="$STATE_DIR/.run-batch-start"
EPOCH_SENTINEL="1970-01-01T00:00:00Z"

MODE=""
FORCE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --write) MODE="write"; shift ;;
    --read)  MODE="read";  shift ;;
    --force) FORCE=1;      shift ;;
    --help|-h)
      printf 'Usage: run-batch-start.sh --write [--force] | --read\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

case "$MODE" in
  write)
    mkdir -p "$STATE_DIR" 2>/dev/null
    if [ -f "$BATCH_FILE" ] && [ "$FORCE" -ne 1 ]; then
      exit 0
    fi
    date -u +%Y-%m-%dT%H:%M:%SZ > "$BATCH_FILE" 2>/dev/null
    exit 0
    ;;
  read)
    if [ -f "$BATCH_FILE" ]; then
      cat "$BATCH_FILE"
    else
      printf '%s\n' "$EPOCH_SENTINEL"
    fi
    exit 0
    ;;
  *)
    printf 'Usage: run-batch-start.sh --write [--force] | --read\n' >&2
    exit 0
    ;;
esac
```

- [ ] **Step 4: Make the script executable and run the test to verify it passes**

```bash
chmod +x skills/autospec-shared/scripts/run-batch-start.sh
bats skills/autospec-shared/tests/unit/run-batch-start.bats
```
Expected: PASS — 7 tests, 0 failures.

- [ ] **Step 5: Wire the BATCH_START write into the autospec-run run-start section (SKILL.md)**

Open `skills/autospec-run/SKILL.md`. The run-start memory-injection section ends at line ~211 (the paragraph ending `...prevent re-occurrence of known pitfalls.`), immediately before `## Phase 4 — Background autonomous monitor`. Append a new subsection after that paragraph and before the `## Phase 4` heading. The exact text to insert (note the leading and trailing blank lines so the surrounding structure is preserved):

```markdown

## Run-start batch timestamp (run-start, once)

Capture the run-start instant so the end-of-run gap-remediation phase (Phase 5.5) can scope its review to work shipped during THIS run. Run exactly once, before the Phase 4 monitor launches:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-batch-start.sh" --write
```

This writes the UTC ISO 8601 timestamp to `~/.autospec/.run-batch-start` (idempotent within a run; pass `--force` only when intentionally starting a fresh batch window). Phase 5.5 reads it back via `run-batch-start.sh --read`, which yields `BATCH_START_DATE`.
```

- [ ] **Step 6: Apply the identical body edit to `codex/prompt.md` and `opencode/agent.md`**

In `skills/autospec-run/codex/prompt.md`, insert the SAME markdown block (byte-identical, from `## Run-start batch timestamp (run-start, once)` through the final paragraph) immediately before its `## Phase 4 — Background autonomous monitor` heading.

In `skills/autospec-run/opencode/agent.md`, insert the SAME markdown block immediately before its `## Phase 4 — Background autonomous monitor` heading.

- [ ] **Step 7: Run the lock-step validator to verify the trio is in sync**

Run: `autospec validate`
Expected: prints `validate: lock-step: autospec-run` with no diff, and ends `validate: OK — all validation checks passed.`

- [ ] **Step 8: Commit**

```bash
git add skills/autospec-shared/scripts/run-batch-start.sh \
        skills/autospec-shared/tests/unit/run-batch-start.bats \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
git commit -m "feat(run): capture run-start BATCH_START timestamp for gap remediation"
```

---

## Task 2: gap-json-lib.sh — shared gap-object schema validator

The gap JSON contract `{gap_id,dimension,severity,file,line,title,body,dedupe_key}` is consumed by the loop driver (Task 3) and asserted by the review-mode tests (Task 4). Putting the schema check + title-hash fallback in one sourceable helper keeps it DRY.

**Files:**
- Create: `skills/autospec-shared/scripts/gap-json-lib.sh`
- Test: covered indirectly via `gap-remediation-loop.bats` (Task 3) and `autospec-review-remediation.bats` (Task 4). This task adds a focused self-test inside the lib's `--help`/`--selftest` path.

- [ ] **Step 1: Write the failing test**

Create `skills/autospec-shared/tests/unit/gap-json-lib.bats`:

```bash
#!/usr/bin/env bats
# gap-json-lib.bats — tests for gap-json-lib.sh schema validation + title-hash.

LIB="${BATS_TEST_DIRNAME}/../../../skills/autospec-shared/scripts/gap-json-lib.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
}
teardown() {
    rm -rf "$TEST_TMP"
}

@test "gap-json-lib.sh --selftest exits 0" {
    run bash "$LIB" --selftest
    [ "$status" -eq 0 ]
}

@test "validate accepts a complete gap object" {
    cat > "$TEST_TMP/gap.json" <<'EOF'
{"gap_id":"G1","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"t","body":"b","dedupe_key":"k1"}
EOF
    run bash "$LIB" --validate-file "$TEST_TMP/gap.json"
    [ "$status" -eq 0 ]
}

@test "validate rejects a gap object missing required keys" {
    cat > "$TEST_TMP/bad.json" <<'EOF'
{"gap_id":"G1","dimension":"correctness"}
EOF
    run bash "$LIB" --validate-file "$TEST_TMP/bad.json"
    [ "$status" -ne 0 ]
}

@test "title-hash is stable and lowercase-hex" {
    run bash "$LIB" --title-hash "cross-repo-search trailing pipe"
    [ "$status" -eq 0 ]
    [[ "$output" =~ ^[0-9a-f]{10}$ ]]
    first="$output"
    run bash "$LIB" --title-hash "cross-repo-search trailing pipe"
    [ "$output" = "$first" ]
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/gap-json-lib.bats`
Expected: FAIL — "gap-json-lib.sh: No such file or directory".

- [ ] **Step 3: Write minimal implementation**

Create `skills/autospec-shared/scripts/gap-json-lib.sh`:

```bash
#!/usr/bin/env bash
# gap-json-lib.sh — shared helpers for the gap JSON contract.
#
# Gap object schema (emitted by `/autospec-review --remediation --emit-gaps`):
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key}
#
# Sourceable OR runnable:
#   bash gap-json-lib.sh --validate-file <path>   # exit 0 if file holds a valid gap object
#   bash gap-json-lib.sh --title-hash "<title>"   # print 10-char sha1 hex (dedupe fallback)
#   bash gap-json-lib.sh --selftest               # internal smoke check
#
# When sourced, exposes:
#   gap_required_keys                  # echoes the required key list
#   gap_validate_object <json-string>  # returns 0/1
#   gap_title_hash <title-string>      # prints 10-char hex
#
# Requires: bash 3.2+, jq, shasum (or sha1sum)

set +e

GAP_REQUIRED_KEYS="gap_id dimension severity file line title body dedupe_key"

gap_required_keys() { printf '%s\n' "$GAP_REQUIRED_KEYS"; }

# gap_validate_object <json-string> — 0 if all required keys present (non-null).
gap_validate_object() {
  local obj="$1" key
  command -v jq >/dev/null 2>&1 || return 1
  printf '%s' "$obj" | jq -e 'type == "object"' >/dev/null 2>&1 || return 1
  for key in $GAP_REQUIRED_KEYS; do
    printf '%s' "$obj" | jq -e --arg k "$key" 'has($k) and (.[$k] != null)' >/dev/null 2>&1 || return 1
  done
  return 0
}

# gap_title_hash <title> — deterministic 10-char hex for dedupe fallback.
gap_title_hash() {
  local title="$1" sum
  if command -v shasum >/dev/null 2>&1; then
    sum="$(printf '%s' "$title" | shasum | awk '{print $1}')"
  else
    sum="$(printf '%s' "$title" | sha1sum | awk '{print $1}')"
  fi
  printf '%s' "${sum:0:10}"
}

# ── Runnable entrypoint (skipped when sourced) ────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  case "${1:-}" in
    --validate-file)
      [ -f "${2:-}" ] || exit 1
      gap_validate_object "$(cat "$2")" || exit 1
      exit 0
      ;;
    --title-hash)
      gap_title_hash "${2:-}"
      printf '\n'
      exit 0
      ;;
    --selftest)
      gap_validate_object '{"gap_id":"G1","dimension":"correctness","severity":"low","file":"a","line":1,"title":"t","body":"b","dedupe_key":"k"}' || exit 1
      gap_validate_object '{"gap_id":"G1"}' && exit 1
      h="$(gap_title_hash "abc")"; [ "${#h}" -eq 10 ] || exit 1
      exit 0
      ;;
    --help|-h)
      printf 'Usage: gap-json-lib.sh --validate-file <path> | --title-hash <title> | --selftest\n'
      exit 0
      ;;
    *)
      printf 'Usage: gap-json-lib.sh --validate-file <path> | --title-hash <title> | --selftest\n' >&2
      exit 0
      ;;
  esac
fi
```

- [ ] **Step 4: Make executable and run the test to verify it passes**

```bash
chmod +x skills/autospec-shared/scripts/gap-json-lib.sh
bats skills/autospec-shared/tests/unit/gap-json-lib.bats
```
Expected: PASS — 4 tests, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/gap-json-lib.sh \
        skills/autospec-shared/tests/unit/gap-json-lib.bats
git commit -m "feat(shared): gap JSON contract validator + title-hash helper"
```

---

## Task 3: gap-remediation-loop.sh — deterministic gap-filing driver

Reads the emitted gap JSON, dedupes against open issues (by `dedupe_key`/title) AND active `docs:drift` labels, files survivors via `gh issue create --label auto-implement,gap-remediation,priority:high`, backfills classify labels, tracks round state in `~/.autospec/gap-round-state.json`, and enforces `AUTOSPEC_GAP_MAX_ROUNDS` (default 2). This is the first `.sh` in the repo to call `gh issue create`; modeled on the classify gh idioms (`gh label create ... --force`, `gh issue edit <N> --add-label`). Because it files via `gh`, it uses `set -eu` + a hard gh-presence guard (per the recon convention for gh-touching scripts), but wraps the per-gap filing in best-effort retry so one bad gap does not abort the batch.

**Files:**
- Create: `skills/autospec-shared/scripts/gap-remediation-loop.sh`
- Test: `skills/autospec-shared/tests/unit/gap-remediation-loop.bats`

- [ ] **Step 1: Write the failing test**

Create `skills/autospec-shared/tests/unit/gap-remediation-loop.bats`:

```bash
#!/usr/bin/env bats
# gap-remediation-loop.bats — tests for gap-remediation-loop.sh
# Covers: dedupe vs open issue, dedupe vs docs:drift, round-cap, convergence,
# skip-flag, malformed/empty JSON.

LOOP="${BATS_TEST_DIRNAME}/../../../skills/autospec-shared/scripts/gap-remediation-loop.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
    export AUTOSPEC_GAP_REPO="testorg/testrepo"
    # Stub gh: records `issue create` calls, serves a configurable open-issue list.
    mkdir -p "$TEST_TMP/bin"
    export GH_CREATE_LOG="$TEST_TMP/gh-create.log"
    export GH_ISSUE_LIST_JSON="$TEST_TMP/open-issues.json"
    printf '[]\n' > "$GH_ISSUE_LIST_JSON"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  *"issue list"*)
    cat "$GH_ISSUE_LIST_JSON"
    exit 0 ;;
  *"label create"*)
    exit 0 ;;
  *"issue create"*)
    # Emit a fake URL ending in an issue number and log the invocation.
    printf '%s\n' "$*" >> "$GH_CREATE_LOG"
    echo "https://github.com/testorg/testrepo/issues/999"
    exit 0 ;;
  *"issue edit"*)
    exit 0 ;;
  *"repo view"*)
    echo "testorg/testrepo"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"

    # A clean gap that has no dedupe collision.
    cat > "$TEST_TMP/gaps.json" <<'EOF'
[{"gap_id":"G1","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"trailing pipe bug","body":"fix it","dedupe_key":"cross-repo-search-trailing-pipe"}]
EOF
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "gap-remediation-loop.sh is executable" {
    [ -x "$LOOP" ]
}

@test "--help exits 0 and prints Usage" {
    run bash "$LOOP" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

@test "files a survivor and prints survivor count 1" {
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=1"* ]]
    grep -q "issue create" "$GH_CREATE_LOG"
    grep -q "auto-implement" "$GH_CREATE_LOG"
    grep -q "gap-remediation" "$GH_CREATE_LOG"
    grep -q "priority:high" "$GH_CREATE_LOG"
}

@test "dedupes against an open issue carrying the same dedupe_key in its body" {
    cat > "$GH_ISSUE_LIST_JSON" <<'EOF'
[{"number":42,"title":"old","body":"dedupe_key: cross-repo-search-trailing-pipe","labels":[{"name":"auto-implement"}]}]
EOF
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "dedupes against an open issue carrying an active docs:drift label with matching title" {
    cat > "$GH_ISSUE_LIST_JSON" <<'EOF'
[{"number":43,"title":"trailing pipe bug","body":"unrelated","labels":[{"name":"docs:drift"}]}]
EOF
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "convergence: empty gap array files nothing and reports survivors=0" {
    printf '[]\n' > "$TEST_TMP/gaps.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
}

@test "malformed JSON is treated as 0 survivors and warns" {
    printf 'not json at all\n' > "$TEST_TMP/gaps.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"WARN"* ]]
}

@test "round-cap: refuses to file when round-state already hit AUTOSPEC_GAP_MAX_ROUNDS" {
    export AUTOSPEC_GAP_MAX_ROUNDS=2
    printf '{"round":2}\n' > "$TEST_TMP/gap-round-state.json"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"round cap"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}

@test "round-state increments after a filing round" {
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/gap-round-state.json"
    [[ "$output" == *'"round": 1'* ]] || [[ "$output" == *'"round":1'* ]]
}

@test "skip flag ~/.autospec/no-review.flag short-circuits to survivors=0" {
    touch "$TEST_TMP/no-review.flag"
    run bash "$LOOP" --gaps "$TEST_TMP/gaps.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=0"* ]]
    [[ "$output" == *"skip"* ]]
    [ ! -f "$GH_CREATE_LOG" ]
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/gap-remediation-loop.bats`
Expected: FAIL — "gap-remediation-loop.sh: No such file or directory".

- [ ] **Step 3: Write minimal implementation**

Create `skills/autospec-shared/scripts/gap-remediation-loop.sh`:

```bash
#!/usr/bin/env bash
# gap-remediation-loop.sh — file surviving gaps from a gap JSON as auto-implement issues.
#
# Reads the gap JSON emitted by `/autospec-review --remediation --emit-gaps <path>`
# (schema: {gap_id,dimension,severity,file,line,title,body,dedupe_key}), dedupes each
# gap against (a) open issues whose body carries the same `dedupe_key` or whose title
# matches, and (b) open issues carrying an active `docs:drift` self-heal label with a
# matching title. Survivors are filed via `gh issue create --label
# auto-implement,gap-remediation,priority:high`, then classified via
# classify-model-fit backfill. Round state is tracked in
# ~/.autospec/gap-round-state.json and capped at AUTOSPEC_GAP_MAX_ROUNDS (default 2).
#
# Usage:
#   gap-remediation-loop.sh --gaps <path> --file    # dedupe + file survivors
#   gap-remediation-loop.sh --help
#
# Environment:
#   AUTOSPEC_STATE_DIR        — state dir (default: ~/.autospec)
#   AUTOSPEC_SCRIPTS_DIR      — sibling scripts dir (default: script dir)
#   AUTOSPEC_GAP_REPO         — repo slug for gh (default: gh repo context)
#   AUTOSPEC_GAP_MAX_ROUNDS   — hard round cap (default: 2)
#
# Output (stdout, last line): "gap-remediation: survivors=<N> filed=<N> round=<N>"
#
# Exit codes:
#   0  always (best-effort; emits WARN on recoverable problems)
#   2  gh CLI absent (hard fail — cannot file issues)
#
# Requires: bash 3.2+, gh, jq

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
MAX_ROUNDS="${AUTOSPEC_GAP_MAX_ROUNDS:-2}"
ROUND_STATE="$STATE_DIR/gap-round-state.json"
SKIP_FLAG="$STATE_DIR/no-review.flag"

GAPS_FILE=""
DO_FILE=0

# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"

while [ $# -gt 0 ]; do
  case "$1" in
    --gaps) GAPS_FILE="$2"; shift 2 ;;
    --file) DO_FILE=1; shift ;;
    --help|-h)
      printf 'Usage: gap-remediation-loop.sh --gaps <path> --file\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

_report() { printf 'gap-remediation: survivors=%s filed=%s round=%s\n' "$1" "$2" "$3"; }

# Hard requirement: gh must exist to file issues.
if ! command -v gh >/dev/null 2>&1; then
  printf 'gap-remediation: ERROR gh CLI not found\n' >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  printf 'gap-remediation: WARN jq not found; treating as 0 survivors\n' >&2
  _report 0 0 0
  exit 0
fi

# ── Read current round (default 0) ────────────────────────────────────────────
_current_round=0
if [ -f "$ROUND_STATE" ]; then
  _current_round="$(jq -r '.round // 0' "$ROUND_STATE" 2>/dev/null || echo 0)"
fi

# ── Skip-flag short-circuit ───────────────────────────────────────────────────
if [ -f "$SKIP_FLAG" ]; then
  printf 'gap-remediation: skip (no-review.flag present)\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi

# ── Round-cap enforcement ─────────────────────────────────────────────────────
if [ "$_current_round" -ge "$MAX_ROUNDS" ]; then
  printf 'gap-remediation: round cap reached (%s/%s); not filing\n' "$_current_round" "$MAX_ROUNDS" >&2
  _report 0 0 "$_current_round"
  exit 0
fi

# ── Parse gaps (malformed/empty → 0 survivors) ────────────────────────────────
if [ -z "$GAPS_FILE" ] || [ ! -f "$GAPS_FILE" ]; then
  printf 'gap-remediation: WARN no gaps file; treating as 0 survivors\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi
if ! jq -e 'type == "array"' "$GAPS_FILE" >/dev/null 2>&1; then
  printf 'gap-remediation: WARN malformed gap JSON; treating as 0 survivors\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi

_gap_count="$(jq 'length' "$GAPS_FILE" 2>/dev/null || echo 0)"
if [ "$_gap_count" -eq 0 ]; then
  _report 0 0 "$_current_round"
  exit 0
fi

# ── Snapshot open issues once (number, title, body, label names) ──────────────
_repo_args=()
[ -n "${AUTOSPEC_GAP_REPO:-}" ] && _repo_args=(--repo "$AUTOSPEC_GAP_REPO")
_open_issues="$(gh issue list "${_repo_args[@]}" --state open --limit 500 \
  --json number,title,body,labels 2>/dev/null || echo '[]')"
echo "$_open_issues" | jq -e 'type == "array"' >/dev/null 2>&1 || _open_issues='[]'

# _is_dup <dedupe_key> <title> → 0 if a matching open issue already exists.
_is_dup() {
  local dk="$1" title="$2"
  # (a) any open issue body contains the dedupe_key, OR title matches exactly.
  echo "$_open_issues" | jq -e --arg dk "$dk" --arg t "$title" '
    any(.[];
      ((.body // "") | contains($dk))
      or (.title == $t)
    )' >/dev/null 2>&1 && return 0
  # (b) any open issue with an active docs:drift label whose title matches.
  echo "$_open_issues" | jq -e --arg t "$title" '
    any(.[];
      (.title == $t)
      and ((.labels // []) | any(.name == "docs:drift"))
    )' >/dev/null 2>&1 && return 0
  return 1
}

# ── Walk gaps, dedupe, file survivors ─────────────────────────────────────────
_filed=0
_survivors=0
_i=0
while [ "$_i" -lt "$_gap_count" ]; do
  _gap="$(jq -c ".[$_i]" "$GAPS_FILE")"
  _i=$((_i + 1))

  # Schema gate — skip invalid objects with a warning.
  if ! gap_validate_object "$_gap"; then
    printf 'gap-remediation: WARN gap %s failed schema; skipping\n' "$_i" >&2
    continue
  fi

  _dk="$(printf '%s' "$_gap" | jq -r '.dedupe_key')"
  _title="$(printf '%s' "$_gap" | jq -r '.title')"
  _body="$(printf '%s' "$_gap" | jq -r '.body')"
  _file="$(printf '%s' "$_gap" | jq -r '.file')"
  _line="$(printf '%s' "$_gap" | jq -r '.line')"
  _dim="$(printf '%s' "$_gap" | jq -r '.dimension')"
  _sev="$(printf '%s' "$_gap" | jq -r '.severity')"

  if _is_dup "$_dk" "$_title"; then
    continue
  fi
  _survivors=$((_survivors + 1))

  [ "$DO_FILE" -eq 1 ] || continue

  # Compose the issue body with the dedupe_key embedded so future rounds match it.
  _issue_body="$(printf '%s\n\n---\n- dimension: %s\n- severity: %s\n- file: %s:%s\n- dedupe_key: %s\n' \
    "$_body" "$_dim" "$_sev" "$_file" "$_line" "$_dk")"

  # Ensure labels exist (idempotent, mirror classify idiom).
  gh label create gap-remediation "${_repo_args[@]}" --color d4c5f9 --force >/dev/null 2>&1 || true
  gh label create priority:high   "${_repo_args[@]}" --color e11d21 --force >/dev/null 2>&1 || true

  # File the issue, retry once on failure (per spec error handling).
  _url=""
  _attempt=0
  while [ "$_attempt" -lt 2 ]; do
    _attempt=$((_attempt + 1))
    _url="$(gh issue create "${_repo_args[@]}" \
      --title "$_title" \
      --body "$_issue_body" \
      --label "auto-implement,gap-remediation,priority:high" 2>/dev/null || true)"
    [ -n "$_url" ] && break
  done

  if [ -z "$_url" ]; then
    printf 'gap-remediation: WARN failed to file gap "%s" after retry\n' "$_title" >&2
    continue
  fi
  _filed=$((_filed + 1))

  # Backfill ctx:*/reasoning:* via classify (best-effort; non-fatal).
  _num="$(printf '%s' "$_url" | grep -oE '[0-9]+$' || true)"
  if [ -n "$_num" ] && [ -x "$AUTOSPEC_SCRIPTS_DIR/classify-model-fit.sh" ]; then
    _tmp_body="$(mktemp)"
    printf '%s' "$_issue_body" > "$_tmp_body"
    _cls="$(bash "$AUTOSPEC_SCRIPTS_DIR/classify-model-fit.sh" "$_tmp_body" --json 2>/dev/null || true)"
    rm -f "$_tmp_body"
    if [ -n "$_cls" ]; then
      _ctx="$(printf '%s' "$_cls" | jq -r '.ctx // empty' 2>/dev/null || true)"
      _rsn="$(printf '%s' "$_cls" | jq -r '.reasoning // empty' 2>/dev/null || true)"
      if [ -n "$_ctx" ] && [ -n "$_rsn" ]; then
        gh issue edit "$_num" "${_repo_args[@]}" \
          --add-label "ctx:$_ctx,reasoning:$_rsn" >/dev/null 2>&1 || true
      fi
    fi
  fi
done

# ── Advance round state when we actually filed something ──────────────────────
if [ "$DO_FILE" -eq 1 ] && [ "$_filed" -gt 0 ]; then
  mkdir -p "$STATE_DIR" 2>/dev/null
  _new_round=$((_current_round + 1))
  printf '{"round": %s, "max_rounds": %s, "last_filed": %s}\n' \
    "$_new_round" "$MAX_ROUNDS" "$_filed" > "$ROUND_STATE"
  _current_round="$_new_round"
fi

_report "$_survivors" "$_filed" "$_current_round"
exit 0
```

- [ ] **Step 4: Make executable and run the test to verify it passes**

```bash
chmod +x skills/autospec-shared/scripts/gap-remediation-loop.sh
bats skills/autospec-shared/tests/unit/gap-remediation-loop.bats
```
Expected: PASS — 10 tests, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/gap-remediation-loop.sh \
        skills/autospec-shared/tests/unit/gap-remediation-loop.bats
git commit -m "feat(shared): gap-remediation-loop deterministic issue-filing driver"
```

---

## Task 4: /autospec-review remediation mode (--remediation, --emit-gaps)

Add the `--remediation` and `--emit-gaps <path>` flags to `/autospec-review`: a broad-dimension review (spec-coverage + correctness + test-quality + integration-wiring + docs), an evaluate-findings/critic false-positive filter before emission, and a machine-readable gap JSON written to the `--emit-gaps` path. All prose changes propagate across the autospec-review trio. A small helper script makes the JSON-shaping testable without invoking the LLM.

**Files:**
- Create: `skills/autospec-shared/scripts/emit-gaps.sh` (shapes filtered findings → gap JSON contract; lets us bats the schema deterministically)
- Create: `skills/autospec-shared/tests/unit/autospec-review-remediation.bats`
- Modify: `skills/autospec-review/SKILL.md` (CLI flags table ~:86; new prose section after `## CLI flags`) + `codex/prompt.md` + `opencode/agent.md`

- [ ] **Step 1: Write the failing test**

Create `skills/autospec-shared/tests/unit/autospec-review-remediation.bats`:

```bash
#!/usr/bin/env bats
# autospec-review-remediation.bats — tests the emit-gaps.sh shaper:
# valid JSON schema emitted; seeded false-positive dropped; seeded broad defect surfaces.

EMIT="${BATS_TEST_DIRNAME}/../../../skills/autospec-shared/scripts/emit-gaps.sh"
LIB="${BATS_TEST_DIRNAME}/../../../skills/autospec-shared/scripts/gap-json-lib.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    # Candidate findings: one genuine correctness defect (keep), one flagged false-positive (drop).
    cat > "$TEST_TMP/findings.json" <<'EOF'
[
  {"dimension":"correctness","severity":"medium","file":"cross-repo-search.sh","line":77,
   "title":"trailing pipe matches every line on BSD grep","body":"build pattern drops trailing \\|","verdict":"keep","dedupe_key":"cross-repo-search-trailing-pipe"},
  {"dimension":"test-quality","severity":"low","file":"x.sh","line":1,
   "title":"phantom defect","body":"reviewer hallucinated","verdict":"false_positive","dedupe_key":"phantom"}
]
EOF
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "emit-gaps.sh is executable" {
    [ -x "$EMIT" ]
}

@test "emits a JSON array where every object satisfies the gap schema" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/gaps.json" ]
    jq -e 'type == "array"' "$TEST_TMP/gaps.json"
    # each element validates against the shared schema
    n="$(jq 'length' "$TEST_TMP/gaps.json")"
    for i in $(seq 0 $((n - 1))); do
        obj="$(jq -c ".[$i]" "$TEST_TMP/gaps.json")"
        run bash -c ". '$LIB'; gap_validate_object '$obj'"
        [ "$status" -eq 0 ]
    done
}

@test "false-positive verdict is dropped by the filter" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq -r '.[].dedupe_key' "$TEST_TMP/gaps.json"
    [[ "$output" == *"cross-repo-search-trailing-pipe"* ]]
    [[ "$output" != *"phantom"* ]]
}

@test "seeded broad-dimension defect (correctness) surfaces as a gap with gap_id" {
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq -r '.[0].gap_id' "$TEST_TMP/gaps.json"
    [ -n "$output" ]
    run jq -r '.[0].dimension' "$TEST_TMP/gaps.json"
    [ "$output" = "correctness" ]
}

@test "empty findings produce an empty array" {
    printf '[]\n' > "$TEST_TMP/findings.json"
    run bash "$EMIT" --findings "$TEST_TMP/findings.json" --out "$TEST_TMP/gaps.json"
    [ "$status" -eq 0 ]
    run jq 'length' "$TEST_TMP/gaps.json"
    [ "$output" = "0" ]
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/autospec-review-remediation.bats`
Expected: FAIL — "emit-gaps.sh: No such file or directory".

- [ ] **Step 3: Write minimal implementation**

Create `skills/autospec-shared/scripts/emit-gaps.sh`:

```bash
#!/usr/bin/env bash
# emit-gaps.sh — shape filtered review findings into the gap JSON contract.
#
# Input findings JSON: array of objects with at least
#   {dimension, severity, file, line, title, body, verdict, dedupe_key}
# where verdict is "keep" or "false_positive" (the evaluate-findings/critic verdict).
# This shaper keeps only verdict=="keep", assigns a stable gap_id (G<n> in order),
# fills a dedupe_key fallback (title-hash) when absent, and writes the gap contract:
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key}
#
# Usage:
#   emit-gaps.sh --findings <path> --out <path>
#   emit-gaps.sh --help
#
# Environment:
#   AUTOSPEC_SCRIPTS_DIR — sibling scripts dir (default: script dir)
#
# Exit codes:
#   0  success (empty/missing input → empty array written)
#   1  jq missing
#
# Requires: bash 3.2+, jq

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"

FINDINGS=""
OUT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --findings) FINDINGS="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --help|-h)
      printf 'Usage: emit-gaps.sh --findings <path> --out <path>\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

command -v jq >/dev/null 2>&1 || { printf 'emit-gaps: jq required\n' >&2; exit 1; }
[ -n "$OUT" ] || { printf 'emit-gaps: --out required\n' >&2; exit 1; }

if [ -z "$FINDINGS" ] || [ ! -f "$FINDINGS" ] || ! jq -e 'type=="array"' "$FINDINGS" >/dev/null 2>&1; then
  printf '[]\n' > "$OUT"
  exit 0
fi

# Keep verdict=="keep" (or missing verdict → treat as keep), shape to contract,
# assign G<n> gap_ids in order, and fill dedupe_key fallback with title-hash.
jq -c '[ .[] | select((.verdict // "keep") == "keep") ]' "$FINDINGS" > "$OUT.tmp"

_n="$(jq 'length' "$OUT.tmp")"
printf '[' > "$OUT"
_i=0
while [ "$_i" -lt "$_n" ]; do
  _f="$(jq -c ".[$_i]" "$OUT.tmp")"
  _gid="G$((_i + 1))"
  _dk="$(printf '%s' "$_f" | jq -r '.dedupe_key // empty')"
  if [ -z "$_dk" ]; then
    _title="$(printf '%s' "$_f" | jq -r '.title // ""')"
    _dk="$(gap_title_hash "$_title")"
  fi
  _obj="$(printf '%s' "$_f" | jq -c --arg gid "$_gid" --arg dk "$_dk" '{
    gap_id: $gid,
    dimension: (.dimension // "correctness"),
    severity: (.severity // "low"),
    file: (.file // ""),
    line: (.line // 0),
    title: (.title // ""),
    body: (.body // ""),
    dedupe_key: $dk
  }')"
  [ "$_i" -gt 0 ] && printf ',' >> "$OUT"
  printf '%s' "$_obj" >> "$OUT"
  _i=$((_i + 1))
done
printf ']\n' >> "$OUT"
rm -f "$OUT.tmp"
exit 0
```

- [ ] **Step 4: Make executable and run the test to verify it passes**

```bash
chmod +x skills/autospec-shared/scripts/emit-gaps.sh
bats skills/autospec-shared/tests/unit/autospec-review-remediation.bats
```
Expected: PASS — 5 tests, 0 failures.

- [ ] **Step 5: Add the two flags to the autospec-review SKILL.md CLI flags table**

In `skills/autospec-review/SKILL.md`, the CLI flags table ends at line ~93 with the `--spec-glob` row. Add two rows immediately after `| \`--spec-glob PATTERN\` | Override default spec discovery globs |`:

```markdown
| `--remediation` | Broad-dimension review (spec-coverage + correctness + test-quality + integration-wiring + docs) with false-positive filter, for the end-of-run gap-remediation loop |
| `--emit-gaps PATH` | Write the machine-readable gap JSON to PATH (implies `--remediation`) |
```

- [ ] **Step 6: Add the remediation-mode prose section to the autospec-review SKILL.md**

In `skills/autospec-review/SKILL.md`, insert a new section immediately AFTER the CLI flags table (after the two rows added in Step 5) and BEFORE `## Phase 0 — Preflight`. Insert this exact block:

```markdown
## Remediation mode (`--remediation` / `--emit-gaps`)

When `--remediation` (or `--emit-gaps PATH`) is passed, run a **broad-dimension** review instead of the spec-coverage-only audit, and emit a machine-readable gap list for the `/autospec-run` end-of-run gap-remediation loop.

**Model tier:** Tier A (spec work) — broad review is inline analysis and needs opus turn-stamina (per `feedback_monitor_silent_exit.md`).

Dimensions reviewed (all five, not just spec-coverage):

1. **spec-coverage** — acceptance criteria with no shipping issue/PR (the existing audit).
2. **correctness** — logic bugs, cross-platform shell portability (e.g. GNU-vs-BSD `grep`/`sed`), off-by-one, error-path gaps.
3. **test-quality** — assertions that never fail, missing negative-path coverage, untested branches.
4. **integration-wiring** — emitted-but-unconsumed artifacts, dangling references, lock-step drift.
5. **docs** — stale or contradictory documentation versus shipped behavior.

Pipeline:

1. Dispatch the broad review subagent(s) at Tier A. Collect candidate findings, each tagged with `dimension`, `severity`, `file`, `line`, `title`, `body`, and a stable `dedupe_key`.
2. **False-positive filter (required before emit):** pipe candidate findings through an evaluate-findings/critic pass. Mark each finding `verdict: keep` or `verdict: false_positive`. When uncertain, drop it (`false_positive`) — never ship a false positive into the auto-implement queue.
3. Write the surviving findings to a temp findings file, then shape them into the gap contract:

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/emit-gaps.sh" \
     --findings /tmp/autospec-review/remediation-findings.json \
     --out "${EMIT_GAPS_PATH:-$HOME/.autospec/gaps-${RUN_ID}.json}"
   ```

   The emitted gap JSON is an array of objects matching the contract:

   ```json
   [{"gap_id":"G1","dimension":"correctness","severity":"medium",
     "file":"skills/autospec-shared/scripts/cross-repo-search.sh","line":77,
     "title":"trailing pipe matches every line on BSD grep",
     "body":"<remediation issue body>",
     "dedupe_key":"cross-repo-search-trailing-pipe"}]
   ```

4. When `--emit-gaps PATH` is given, write to PATH; otherwise default to `~/.autospec/gaps-<run_id>.json`. The driver (`gap-remediation-loop.sh`) reads this file. On review-subagent failure, write an empty array (`[]`) so the driver converges cleanly, and log a warning — never block run completion.
```

- [ ] **Step 7: Apply Steps 5–6 byte-identically to the autospec-review trio**

In `skills/autospec-review/codex/prompt.md`: add the same two CLI-flag rows after the `--spec-glob` row (codex table at ~:86), then insert the same `## Remediation mode (...)` section before its `## Phase 0 — Preflight` heading.

In `skills/autospec-review/opencode/agent.md`: add the same two CLI-flag rows after the `--spec-glob` row (opencode table at ~:90), then insert the same `## Remediation mode (...)` section before its `## Phase 0 — Preflight` heading.

- [ ] **Step 8: Run the lock-step validator**

Run: `autospec validate`
Expected: prints `autospec-review opencode lockstep` / `autospec-review codex lockstep` checks passing (`check_autospec_review_skill_present`), and `autospec-review Tier A directives` passing (`check_autospec_review_tier_a_directives` requires ≥2 `Tier A (spec work)` directives; the review SKILL.md already has 2, and Step 6 adds a third — still ≥2). Ends `validate: OK — all validation checks passed.`

- [ ] **Step 9: Commit**

```bash
git add skills/autospec-shared/scripts/emit-gaps.sh \
        skills/autospec-shared/tests/unit/autospec-review-remediation.bats \
        skills/autospec-review/SKILL.md \
        skills/autospec-review/codex/prompt.md \
        skills/autospec-review/opencode/agent.md
git commit -m "feat(review): remediation mode — broad review + filter + gap JSON emit"
```

---

## Task 5: /autospec-run Phase 5.5 — bounded gap-remediation loop

Replace the report-only `## Post-batch audit (autospec-review interlock)` section (SKILL.md:831-850) with `## Phase 5.5 — End-of-run gap remediation`, placed between `## Phase 6 — Final report` (ends ~:829) and `## Constraints (apply throughout)` (:852). The new section drives the bounded loop: review → file → monitor-drain → re-review, ≤ `AUTOSPEC_GAP_MAX_ROUNDS`, converge on 0 survivors, honor skip flags, feed Phase 6 report. Propagated across the autospec-run trio.

**Files:**
- Modify: `skills/autospec-run/SKILL.md` (replace :831-850) + `codex/prompt.md` (replace :826-846) + `opencode/agent.md` (replace :830-850)

- [ ] **Step 1: Replace the Post-batch audit section in SKILL.md**

In `skills/autospec-run/SKILL.md`, delete the entire block from the heading `## Post-batch audit (autospec-review interlock)` (line 831) through the line `overall run.` (line 850, the last line before `## Constraints (apply throughout)`). Replace it with this exact section:

```markdown
## Phase 5.5 — End-of-run gap remediation

Runs after the last issue in the batch closes/merges (queue drains, `ALL_DONE`), before the final report. Replaces the old report-only post-batch audit: it broad-reviews the shipped work, files surviving gaps as `auto-implement` issues, and re-runs the monitor to close them — bounded by a round cap.

**Skip the whole phase when:**

- `~/.autospec/no-review.flag` exists, OR
- `--no-postreview` was passed to autospec-run.

Otherwise run the bounded loop:

```bash
BATCH_START_DATE="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-batch-start.sh" --read)"
RUN_ID="$(date -u +%Y%m%dT%H%MZ)-$(git rev-parse --short HEAD)"
GAPS_FILE="$HOME/.autospec/gaps-${RUN_ID}.json"
MAX="${AUTOSPEC_GAP_MAX_ROUNDS:-2}"
rm -f "$HOME/.autospec/gap-round-state.json"   # fresh window per run
```

Loop (`round = 1 … MAX`):

1. **Broad review** (Tier A): run `/autospec-review --remediation --since "${BATCH_START_DATE}" --emit-gaps "${GAPS_FILE}"`. On review failure, log a warning, treat as 0 survivors, and fall back to the final report (never block run completion).
2. **File survivors:**

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gap-remediation-loop.sh" \
     --gaps "${GAPS_FILE}" --file
   ```

   The driver prints `gap-remediation: survivors=<N> filed=<N> round=<N>`. Capture `<N>` survivors.
3. **Converge:** if `survivors == 0`, break — the run is clean.
4. **Drain:** otherwise re-enter the Phase 4 background monitor (opus, `batch=1`) and run it until the freshly-filed `gap-remediation` issues drain (same monitor batch-exit discipline as the main run). `gap-remediation`-labelled issues are recognizable so a later round does not re-flag freshly-fixed work.
5. Increment `round`. The driver also enforces `AUTOSPEC_GAP_MAX_ROUNDS` internally (it refuses to file once the round-state hits the cap), so the loop never spins.

**Termination guarantees:** convergence (0 survivors) ends the loop immediately; the `dedupe_key` prevents re-filing the same gap across rounds; the hard cap `AUTOSPEC_GAP_MAX_ROUNDS` (default 2) stops the loop and surfaces any remainder to the operator.

**Feed Phase 6:** report (a) gaps closed this phase, (b) findings the filter suppressed, and (c) gaps still open after the cap. Failures from `/autospec-review` or the driver log a warning but do NOT fail the overall run.
```

- [ ] **Step 2: Apply the identical replacement to `codex/prompt.md`**

In `skills/autospec-run/codex/prompt.md`, delete the block from `## Post-batch audit (autospec-review interlock)` (line ~826) through its last line before `## Constraints (apply throughout)` (line ~846), and insert the SAME `## Phase 5.5 — End-of-run gap remediation` section (byte-identical to Step 1).

- [ ] **Step 3: Apply the identical replacement to `opencode/agent.md`**

In `skills/autospec-run/opencode/agent.md`, delete the block from `## Post-batch audit (autospec-review interlock)` (line ~830) through its last line before `## Constraints (apply throughout)` (line ~851), and insert the SAME `## Phase 5.5 — End-of-run gap remediation` section (byte-identical to Step 1).

- [ ] **Step 4: Run the lock-step validator**

Run: `autospec validate`
Expected: `validate: lock-step: autospec-run` passes with no diff. Note: this will still pass `check_subagent_model_tier` for autospec-run because the replacement adds no new `**Model tier:**` directive (the per-skill count for autospec-run stays A=2, B=3). Ends `validate: OK — all validation checks passed.`

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
git commit -m "feat(run): Phase 5.5 bounded end-of-run gap-remediation loop"
```

---

## Task 6: validate.sh named-section checks for the new SKILL.md sections

Clone `check_stop_mode_section()` (validate.sh:123-135) into two new cross-trio section checks — one asserting the autospec-run trio carries `## Phase 5.5 — End-of-run gap remediation`, one asserting the autospec-review trio carries `## Remediation mode`. Register both at the call site near `check_stop_mode_section` (validate.sh:636).

**Files:**
- Modify: `autospec validate` (add two functions after `check_stop_mode_section`; add two calls in `main()`)

- [ ] **Step 1: Add the two check functions**

In `autospec validate`, immediately after the closing `}` of `check_stop_mode_section()` (line 133), insert these two functions:

```bash
# Phase 5.5 gap-remediation section invariant: the autospec-run trio must carry
# a `## Phase 5.5 — End-of-run gap remediation` heading in all three trio files.
# Cloned from check_stop_mode_section.
check_gap_remediation_section() {
    skill_dir="skills/autospec-run"
    [ -d "$skill_dir" ] || return 0
    info "gap-remediation section: autospec-run"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Phase 5.5 — End-of-run gap remediation' "$skill_dir/$trio" \
            || fail "autospec-run: $trio missing '## Phase 5.5 — End-of-run gap remediation' section"
    done
}

# Remediation-mode section invariant: the autospec-review trio must carry a
# `## Remediation mode` heading in all three trio files. Cloned from
# check_stop_mode_section.
check_review_remediation_section() {
    skill_dir="skills/autospec-review"
    [ -d "$skill_dir" ] || return 0
    info "remediation-mode section: autospec-review"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Remediation mode' "$skill_dir/$trio" \
            || fail "autospec-review: $trio missing '## Remediation mode' section"
    done
}
```

- [ ] **Step 2: Register the two checks in main()**

In `autospec validate`, find the call `check_stop_mode_section` inside `main()` (line 636) and add the two new calls immediately after it:

```bash
    check_stop_mode_section
    check_gap_remediation_section
    check_review_remediation_section
```

- [ ] **Step 3: Run validate to verify the new checks pass against the work from Tasks 4–5**

Run: `autospec validate`
Expected: prints `validate: gap-remediation section: autospec-run` and `validate: remediation-mode section: autospec-review`, then ends `validate: OK — all validation checks passed.`

- [ ] **Step 4: Negative-check the new guard catches drift (sanity, then revert)**

```bash
cp autospec validate /tmp/validate.sh.bak
# Temporarily break the assertion target to prove the guard fires:
perl -0pi -e 's/## Phase 5.5 — End-of-run gap remediation/## Phase 5.5 — TYPO/' skills/autospec-run/SKILL.md
autospec validate; echo "exit=$?"
```
Expected: FAILS with `autospec-run: SKILL.md missing '## Phase 5.5 — End-of-run gap remediation' section` (and a nonzero exit). Then revert:

```bash
perl -0pi -e 's/## Phase 5.5 — TYPO/## Phase 5.5 — End-of-run gap remediation/' skills/autospec-run/SKILL.md
autospec validate; echo "exit=$?"
```
Expected: `validate: OK — all validation checks passed.` and `exit=0`.

- [ ] **Step 5: Commit**

```bash
git add autospec validate
git commit -m "test(validate): named-section checks for Phase 5.5 + remediation mode"
```

---

## Task 7: Final integration — full validate + full bats green

Confirm the whole change set passes the repository validation harness and every bats suite, including the new ones.

**Files:** none (verification only).

- [ ] **Step 1: Run the full repository validator**

Run: `autospec validate`
Expected: a stream of `validate: ...` lines ending with `validate: OK — all validation checks passed.` and exit code 0.

- [ ] **Step 2: Run the new shared-script bats suites together**

Run: `bats skills/autospec-shared/tests/unit/run-batch-start.bats skills/autospec-shared/tests/unit/gap-json-lib.bats skills/autospec-shared/tests/unit/gap-remediation-loop.bats skills/autospec-shared/tests/unit/autospec-review-remediation.bats`
Expected: `26 tests, 0 failures` (7 + 4 + 10 + 5).

- [ ] **Step 3: Run the full bats tree to confirm no regressions**

Run: `bats --recursive skills/autospec-shared/tests tests`
Expected: all tests pass, 0 failures. (If a pre-existing suite is red for unrelated reasons, confirm via `git stash` that it was red before this change set; do not let a pre-existing failure block this plan.)

- [ ] **Step 4: Confirm bash syntax of all new scripts**

Run: `for f in run-batch-start gap-json-lib gap-remediation-loop emit-gaps; do bash -n "skills/autospec-shared/scripts/$f.sh" && echo "OK $f"; done`
Expected: four `OK ...` lines, exit 0.

- [ ] **Step 5: Commit (no-op safety / final marker)**

```bash
git status --short
# If anything is uncommitted from verification, stage and commit it:
git add -A && git commit -m "chore: end-of-run gap remediation — integration verified" || echo "nothing to commit"
```

---

## Execution note

Because the user wants autospec used maximally, the RECOMMENDED execution is NOT manual subagent dispatch but to **file these seven tasks as `auto-implement` issues and drain them through the autospec monitor loop** (opus, `batch=1`). File one issue per task, each carrying the task's full TDD steps and code as the body, with **Depends-on** links so the queue respects ordering: Task 2 and Task 1 are independent and can run first; Task 3 depends on Task 2 (sources `gap-json-lib.sh`); Task 4 depends on Task 2 (sources `gap-json-lib.sh`); **Task 4 (Phase 5.5 wiring) depends on Tasks 1–3** because Phase 5.5 reads `run-batch-start.sh` (Task 1), calls `gap-remediation-loop.sh` (Task 3), and invokes the review remediation mode (Task 4's emitter) — in this plan Phase 5.5 is Task 5, so **Task 5 Depends-on Tasks 1, 3, 4**; Task 6 (validate checks) Depends-on Tasks 4 and 5; Task 7 Depends-on all. Run `/autospec-classify` to backfill `ctx:*`/`reasoning:*` on the filed issues, then `/autospec-run` to drain. This dogfoods the very gap-remediation loop this plan builds.
