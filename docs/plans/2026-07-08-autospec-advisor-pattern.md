# Autospec Advisor Pattern Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an on-demand strong-model advisor escalation layer to autospec's Phase 4 loop — a cheap executor escalates bounded hard decisions to a harness-native TIER_A advisor that returns advice-only guidance.

**Architecture:** A single deterministic shell primitive (`advisor-escalate.sh`) owns all bookkeeping — flag/gate gating, per-issue cap, context curation, response validation, telemetry — and never calls a model. The model call itself is a prose contract the executor follows via a capability-ordered dispatch ladder (native advisor tool → TIER_A subagent → CLI fallback). Four escalation gates share the one primitive; all are behind `AUTOSPEC_ADVISOR` (default `off`), with `impl-haiku` enabled first behind a cost/quality promotion gate.

**Tech Stack:** Bash (POSIX-ish, must run under macOS bash 3.2), `jq` for JSON, `bats` for tests. Multi-harness skill trios (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`) derived via `derive-trio.sh` / `gen-skill-goldens.sh`, gated by `validate.sh`.

## Global Constraints

- **Source spec:** `docs/specs/2026-07-08-autospec-advisor-pattern-design.md` — authoritative for all behavior.
- **Fail-open everywhere:** any script error, missing dep, cap-reached, or disabled flag → the gate no-ops and Phase 4 proceeds exactly as today. Never block an issue on the advisor path.
- **Script never calls a model.** `advisor-escalate.sh` does bookkeeping only; dispatch is prose.
- **Path-scoped state:** per-issue state at `${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}/<repo-slug>/<issue>.json`, where `<repo-slug>` = `<owner/repo>` with `/`→`_` (no cross-repo collision).
- **Cap default:** `AUTOSPEC_ADVISOR_MAX_USES=3`, shared across all four gates per issue.
- **Guidance budget:** `AUTOSPEC_ADVISOR_MAX_CHARS=2800` (≈700 tokens); over-budget → truncate + `over_budget:true`.
- **Flags:** `AUTOSPEC_ADVISOR` (default `off`); `AUTOSPEC_ADVISOR_GATES` (default `impl-haiku`) — comma list of enabled gate ids.
- **Bash gotchas (all previously bit this repo):** one-sided conditionals use `if/then/fi` not `[ x ] && y` (aborts under `set -e`); no `RETURN` traps (leak under `set -u`); write a real temp file before any `[ -f <(...) ]` (bash 3.2); parse a gate's own final status line, never a background pipeline's exit.
- **Exit codes — precheck:** `0`=GO · `7`=cap-reached · `8`=gate-disabled. **record:** `0`=valid guidance · `2`=invalid/fail-open. Callers treat any non-zero as "proceed without advisor."
- **Install:** new runtime scripts MUST be added to `install.sh` explicitly (installer has a known gap dropping runtime scripts; ship-completeness does not catch it).
- **Trio discipline:** author `SKILL.md` only, then `derive-trio.sh --in-place` + `gen-skill-goldens.sh`; a new section heading requires updating `validate.sh` named-content checks in the same commit. Trio-prose edits and goldens regen land in ONE commit.

---

### Task 1: `advisor-escalate.sh` — precheck phase (gating + cap + curation)

**Files:**
- Create: `skills/autospec-shared/scripts/advisor-escalate.sh`
- Test: `skills/autospec-shared/tests/unit/advisor-escalate.bats`

**Interfaces:**
- Consumes: nothing (leaf primitive).
- Produces: CLI `advisor-escalate.sh --phase precheck --issue <N> --repo <owner/repo> --gate <id> --question-file <f> --context-file <f> [--json]`. On GO (exit 0) prints a JSON object `{"decision":"GO","harness":"<h>","cli_fallback":"<cmd>","use_count":<n>,"payload_file":"<path>"}` where `payload_file` holds the concatenated question+context to hand the advisor. Exit `7` prints `{"decision":"CAP-REACHED"}`; exit `8` prints `{"decision":"DISABLED"}`.

- [ ] **Step 1: Write the failing tests**

```bash
# skills/autospec-shared/tests/unit/advisor-escalate.bats
setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-escalate.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_ADVISOR_STATE_DIR="$TMP/state"
  export AUTOSPEC_ADVISOR=on
  export AUTOSPEC_ADVISOR_GATES=impl-haiku,retry,reviewer,impl-decision
  export AUTOSPEC_HARNESS=claude
  printf 'How should I resolve the ambiguous contract?' > "$TMP/q.txt"
  printf 'Issue #42 context here.' > "$TMP/c.txt"
}
teardown() { rm -rf "$TMP"; }

@test "precheck GO on first use emits decision GO and use_count 1" {
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"decision":"GO"'
  echo "$output" | grep -q '"use_count":1'
}

@test "precheck disabled when AUTOSPEC_ADVISOR=off exits 8" {
  export AUTOSPEC_ADVISOR=off
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 8 ]
  echo "$output" | grep -q '"decision":"DISABLED"'
}

@test "precheck disabled when gate not in AUTOSPEC_ADVISOR_GATES exits 8" {
  export AUTOSPEC_ADVISOR_GATES=impl-haiku
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate reviewer --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 8 ]
}

@test "precheck cap-reached after MAX_USES exits 7" {
  export AUTOSPEC_ADVISOR_MAX_USES=2
  for _ in 1 2; do
    bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
      --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  done
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-decision --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 7 ]
  echo "$output" | grep -q '"decision":"CAP-REACHED"'
}

@test "cap is shared across gates for one issue" {
  export AUTOSPEC_ADVISOR_MAX_USES=1
  bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate reviewer --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 7 ]
}

@test "state is path-scoped: same issue number, two repos, no collision" {
  export AUTOSPEC_ADVISOR_MAX_USES=1
  bash "$SCRIPT" --phase precheck --issue 7 --repo acme/one \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  run bash "$SCRIPT" --phase precheck --issue 7 --repo acme/two \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"use_count":1'
}

@test "precheck emits harness-specific cli_fallback for codex" {
  export AUTOSPEC_HARNESS=codex
  run bash "$SCRIPT" --phase precheck --issue 42 --repo acme/widget \
    --gate impl-haiku --question-file "$TMP/q.txt" --context-file "$TMP/c.txt" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"harness":"codex"'
  echo "$output" | grep -q 'codex exec'
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bats skills/autospec-shared/tests/unit/advisor-escalate.bats`
Expected: FAIL — script does not exist (`No such file or directory`).

- [ ] **Step 3: Write the precheck implementation**

```bash
#!/usr/bin/env bash
# advisor-escalate.sh — deterministic bookkeeping for the autospec advisor pattern.
# The model call is NOT made here; dispatch is the executor's prose contract.
# See docs/specs/2026-07-08-autospec-advisor-pattern-design.md.
set -eu

PHASE="" ISSUE="" REPO="" GATE=""
QUESTION_FILE="" CONTEXT_FILE="" RESPONSE_FILE=""
JSON_MODE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --phase) PHASE="${2:-}"; shift 2 ;;
    --issue) ISSUE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --gate) GATE="${2:-}"; shift 2 ;;
    --question-file) QUESTION_FILE="${2:-}"; shift 2 ;;
    --context-file) CONTEXT_FILE="${2:-}"; shift 2 ;;
    --response-file) RESPONSE_FILE="${2:-}"; shift 2 ;;
    --json) JSON_MODE=1; shift ;;
    --help|-h) printf 'Usage: advisor-escalate.sh --phase precheck|record ...\n'; exit 0 ;;
    *) printf 'advisor-escalate.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

STATE_ROOT="${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}"
MAX_USES="${AUTOSPEC_ADVISOR_MAX_USES:-3}"
MAX_CHARS="${AUTOSPEC_ADVISOR_MAX_CHARS:-2800}"
ENABLED="${AUTOSPEC_ADVISOR:-off}"
GATES="${AUTOSPEC_ADVISOR_GATES:-impl-haiku}"
HARNESS="${AUTOSPEC_HARNESS:-claude}"

repo_slug() { printf '%s' "$1" | tr '/' '_'; }

gate_enabled() {
  # word-boundary membership test on a comma list, no regex injection
  case ",${GATES}," in
    *",${GATE},"*) return 0 ;;
    *) return 1 ;;
  esac
}

cli_fallback_for() {
  case "$1" in
    codex) printf 'codex exec' ;;
    opencode) printf 'opencode run' ;;
    *) printf 'claude -p --model opus' ;;
  esac
}

state_file() {
  printf '%s/%s/%s.json' "$STATE_ROOT" "$(repo_slug "$REPO")" "$ISSUE"
}

do_precheck() {
  if [ "$ENABLED" != "on" ]; then
    printf '{"decision":"DISABLED"}\n'
    exit 8
  fi
  if ! gate_enabled; then
    printf '{"decision":"DISABLED"}\n'
    exit 8
  fi

  local sf; sf="$(state_file)"
  local dir; dir="$(dirname "$sf")"
  mkdir -p "$dir"

  local used=0
  if [ -f "$sf" ]; then
    used="$(jq -r '.use_count // 0' "$sf" 2>/dev/null || printf '0')"
  fi

  if [ "$used" -ge "$MAX_USES" ]; then
    printf '{"decision":"CAP-REACHED"}\n'
    exit 7
  fi

  local next=$((used + 1))
  printf '{"use_count":%d}\n' "$next" > "$sf"

  # Curate a decision-scoped payload: question first, then context.
  local payload; payload="$(mktemp -t advisor-payload-XXXXXX)"
  {
    printf '## Question\n'
    cat "$QUESTION_FILE"
    printf '\n\n## Context\n'
    cat "$CONTEXT_FILE"
  } > "$payload"

  jq -cn --arg h "$HARNESS" --arg cli "$(cli_fallback_for "$HARNESS")" \
        --argjson uc "$next" --arg pf "$payload" \
    '{decision:"GO",harness:$h,cli_fallback:$cli,use_count:$uc,payload_file:$pf}'
  exit 0
}

case "$PHASE" in
  precheck) do_precheck ;;
  record) do_record ;;   # defined in Task 2
  *) printf 'advisor-escalate.sh: --phase must be precheck or record\n' >&2; exit 1 ;;
esac
```

Note: `do_record` is added in Task 2; until then the `record` branch is unreachable in these tests.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bats skills/autospec-shared/tests/unit/advisor-escalate.bats`
Expected: PASS (all precheck tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/advisor-escalate.sh skills/autospec-shared/tests/unit/advisor-escalate.bats
git commit -m "feat: advisor-escalate.sh precheck phase (gating, cap, curation)"
```

---

### Task 2: `advisor-escalate.sh` — record phase (validation + telemetry)

**Files:**
- Modify: `skills/autospec-shared/scripts/advisor-escalate.sh` (add `do_record`)
- Modify: `skills/autospec-shared/tests/unit/advisor-escalate.bats` (add record tests)

**Interfaces:**
- Consumes: precheck (Task 1) for the cap/state lifecycle.
- Produces: CLI `advisor-escalate.sh --phase record --issue <N> --repo <owner/repo> --gate <id> --response-file <f> [--json]`. Prints validated `{"verdict":"plan|correction|stop","guidance":"...","over_budget":bool}`. Exit `0` on valid, `2` on unparseable (prints `{"verdict":"stop","guidance":"advisor response unparseable; fail-safe stop","over_budget":false}`). Appends one line to `.autospec/telemetry/advisor-escalate.jsonl`.

- [ ] **Step 1: Write the failing tests**

```bash
# append to advisor-escalate.bats
@test "record validates a well-formed plan verdict" {
  printf '{"verdict":"plan","guidance":"Do X then Y.","confidence":"high"}' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"verdict":"plan"'
  echo "$output" | grep -q '"over_budget":false'
}

@test "record truncates over-budget guidance and flags over_budget" {
  export AUTOSPEC_ADVISOR_MAX_CHARS=20
  printf '{"verdict":"correction","guidance":"%s"}' "$(printf 'x%.0s' $(seq 1 100))" > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate retry --response-file "$TMP/r.json" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"over_budget":true'
}

@test "record fail-safes to stop on unparseable response, exit 2" {
  printf 'not json at all' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 2 ]
  echo "$output" | grep -q '"verdict":"stop"'
}

@test "record rejects an invalid verdict value, fail-safe stop" {
  printf '{"verdict":"proceed","guidance":"go"}' > "$TMP/r.json"
  run bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate impl-haiku --response-file "$TMP/r.json" --json
  [ "$status" -eq 2 ]
  echo "$output" | grep -q '"verdict":"stop"'
}

@test "record appends one telemetry line with gate and verdict" {
  export AUTOSPEC_TELEMETRY_DIR="$TMP/telem"
  printf '{"verdict":"plan","guidance":"ok"}' > "$TMP/r.json"
  bash "$SCRIPT" --phase record --issue 42 --repo acme/widget \
    --gate reviewer --response-file "$TMP/r.json" --json
  [ -f "$TMP/telem/advisor-escalate.jsonl" ]
  grep -q '"gate":"reviewer"' "$TMP/telem/advisor-escalate.jsonl"
  grep -q '"verdict":"plan"' "$TMP/telem/advisor-escalate.jsonl"
  [ "$(wc -l < "$TMP/telem/advisor-escalate.jsonl")" -eq 1 ]
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bats skills/autospec-shared/tests/unit/advisor-escalate.bats`
Expected: FAIL — `do_record` not defined / record branch errors.

- [ ] **Step 3: Implement `do_record`**

Add this function above the final `case "$PHASE"` block:

```bash
telemetry_file() {
  local dir="${AUTOSPEC_TELEMETRY_DIR:-.autospec/telemetry}"
  mkdir -p "$dir"
  printf '%s/advisor-escalate.jsonl' "$dir"
}

emit_stop_failsafe() {
  local reason="$1"
  jq -cn --arg g "$reason" '{verdict:"stop",guidance:$g,over_budget:false}'
}

do_record() {
  local ts; ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date +%Y-%m-%dT%H:%M:%SZ)"
  local tf; tf="$(telemetry_file)"

  # Parse + validate. Any failure → fail-safe stop, exit 2.
  local verdict guidance
  verdict="$(jq -r '.verdict // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"
  guidance="$(jq -r '.guidance // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"

  case "$verdict" in
    plan|correction|stop) : ;;
    *)
      emit_stop_failsafe "advisor response unparseable; fail-safe stop"
      printf '{"ts":"%s","issue":"%s","repo":"%s","gate":"%s","verdict":"stop","tokens_in":0,"tokens_out":0,"over_budget":false,"failsafe":true}\n' \
        "$ts" "$ISSUE" "$REPO" "$GATE" >> "$tf"
      exit 2
      ;;
  esac

  # Strip anything that looks like a tool call or user-facing output (advice-only).
  guidance="$(printf '%s' "$guidance" | sed -e 's/<[^>]*tool[^>]*>//g')"

  # Budget: char-count proxy (~4 chars/token).
  local over=false
  local len=${#guidance}
  if [ "$len" -gt "$MAX_CHARS" ]; then
    guidance="${guidance:0:$MAX_CHARS}"
    over=true
  fi
  local out_len=${#guidance}

  jq -cn --arg v "$verdict" --arg g "$guidance" --argjson ob "$over" \
    '{verdict:$v,guidance:$g,over_budget:$ob}'

  printf '{"ts":"%s","issue":"%s","repo":"%s","gate":"%s","verdict":"%s","tokens_in":0,"tokens_out":%d,"over_budget":%s}\n' \
    "$ts" "$ISSUE" "$REPO" "$GATE" "$verdict" "$out_len" "$over" >> "$tf"
  exit 0
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `bats skills/autospec-shared/tests/unit/advisor-escalate.bats`
Expected: PASS (all precheck + record tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/advisor-escalate.sh skills/autospec-shared/tests/unit/advisor-escalate.bats
git commit -m "feat: advisor-escalate.sh record phase (validation, budget, telemetry)"
```

---

### Task 3: `advisor-report.sh` — promotion-gate report

**Files:**
- Create: `skills/autospec-shared/scripts/advisor-report.sh`
- Test: `skills/autospec-shared/tests/unit/advisor-report.bats`

**Interfaces:**
- Consumes: the telemetry JSONL written by Task 2 (`advisor-escalate.jsonl`), plus a baseline LGTM-first-pass rate and Sonnet-solo cost passed as flags.
- Produces: CLI `advisor-report.sh --telemetry <jsonl> [--json]`. Prints a per-gate summary `{gate: {calls, plan, correction, stop, over_budget, total_tokens_out}}` and, when `--baseline-lgtm <float> --observed-lgtm <float> --baseline-cost <n> --observed-cost <n>` are supplied, a `promote` boolean = (observed-lgtm ≥ baseline-lgtm AND observed-cost ≤ baseline-cost).

- [ ] **Step 1: Write the failing tests**

```bash
# skills/autospec-shared/tests/unit/advisor-report.bats
setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/advisor-report.sh"
  TMP="$(mktemp -d)"
  cat > "$TMP/t.jsonl" <<'EOF'
{"ts":"t","issue":"1","repo":"a/b","gate":"impl-haiku","verdict":"plan","tokens_out":100,"over_budget":false}
{"ts":"t","issue":"2","repo":"a/b","gate":"impl-haiku","verdict":"stop","tokens_out":50,"over_budget":false}
{"ts":"t","issue":"3","repo":"a/b","gate":"reviewer","verdict":"plan","tokens_out":80,"over_budget":true}
EOF
}
teardown() { rm -rf "$TMP"; }

@test "report counts calls per gate" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.["impl-haiku"].calls == 2' >/dev/null
  echo "$output" | jq -e '.["reviewer"].calls == 1' >/dev/null
}

@test "promote true when quality up and cost down" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.72 --baseline-cost 1000 --observed-cost 900 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == true' >/dev/null
}

@test "promote false when cost regressed" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.75 --baseline-cost 1000 --observed-cost 1100 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == false' >/dev/null
}

@test "promote false when quality regressed" {
  run bash "$SCRIPT" --telemetry "$TMP/t.jsonl" \
    --baseline-lgtm 0.70 --observed-lgtm 0.65 --baseline-cost 1000 --observed-cost 800 --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.promote == false' >/dev/null
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bats skills/autospec-shared/tests/unit/advisor-report.bats`
Expected: FAIL — script does not exist.

- [ ] **Step 3: Write the implementation**

```bash
#!/usr/bin/env bash
# advisor-report.sh — summarize advisor telemetry + evaluate the promotion gate.
set -eu

TELEMETRY="" JSON_MODE=0
BL_LGTM="" OB_LGTM="" BL_COST="" OB_COST=""

while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --baseline-lgtm) BL_LGTM="${2:-}"; shift 2 ;;
    --observed-lgtm) OB_LGTM="${2:-}"; shift 2 ;;
    --baseline-cost) BL_COST="${2:-}"; shift 2 ;;
    --observed-cost) OB_COST="${2:-}"; shift 2 ;;
    --json) JSON_MODE=1; shift ;;
    --help|-h) printf 'Usage: advisor-report.sh --telemetry <jsonl> [gate flags]\n'; exit 0 ;;
    *) printf 'advisor-report.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

if [ -z "$TELEMETRY" ] || [ ! -f "$TELEMETRY" ]; then
  printf 'advisor-report.sh: telemetry file not found: %s\n' "$TELEMETRY" >&2
  exit 1
fi

# Per-gate aggregation via jq.
summary="$(jq -s '
  group_by(.gate) | map({
    key: .[0].gate,
    value: {
      calls: length,
      plan: (map(select(.verdict=="plan")) | length),
      correction: (map(select(.verdict=="correction")) | length),
      stop: (map(select(.verdict=="stop")) | length),
      over_budget: (map(select(.over_budget==true)) | length),
      total_tokens_out: (map(.tokens_out // 0) | add)
    }
  }) | from_entries
' "$TELEMETRY")"

# Promotion gate arithmetic (only when all four baselines are present).
promote_obj="{}"
if [ -n "$BL_LGTM" ] && [ -n "$OB_LGTM" ] && [ -n "$BL_COST" ] && [ -n "$OB_COST" ]; then
  promote="$(jq -n \
    --argjson bl "$BL_LGTM" --argjson ol "$OB_LGTM" \
    --argjson bc "$BL_COST" --argjson oc "$OB_COST" \
    '($ol >= $bl) and ($oc <= $bc)')"
  promote_obj="$(jq -n --argjson p "$promote" '{promote:$p}')"
fi

jq -n --argjson s "$summary" --argjson p "$promote_obj" '$s + $p'
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `bats skills/autospec-shared/tests/unit/advisor-report.bats`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/advisor-report.sh skills/autospec-shared/tests/unit/advisor-report.bats
git commit -m "feat: advisor-report.sh telemetry summary + promotion gate"
```

---

### Task 4: Ship the scripts in `install.sh`

**Files:**
- Modify: `install.sh` (add the two scripts to whatever copies `skills/autospec-shared/scripts/*` into `~/.autospec/scripts/`)
- Test: `skills/autospec-shared/tests/unit/advisor-install.bats`

**Interfaces:**
- Consumes: the two scripts from Tasks 1–3.
- Produces: after `install.sh`, both scripts resolve at `~/.autospec/scripts/`.

- [ ] **Step 1: Inspect how install.sh copies shared scripts**

Run: `grep -n 'autospec-shared/scripts\|\.autospec/scripts' install.sh`
Read the surrounding block. If it globs `scripts/*.sh`, the new files are already covered — verify with the smoke test below and skip Step 3. If it enumerates files explicitly, add the two names in Step 3.

- [ ] **Step 2: Write the failing smoke test**

```bash
# skills/autospec-shared/tests/unit/advisor-install.bats
@test "advisor scripts are staged by install.sh into a target dir" {
  TMP="$(mktemp -d)"
  export AUTOSPEC_HOME="$TMP/home"   # if install.sh honors a target override; else DESTDIR
  run bash "${BATS_TEST_DIRNAME}/../../../../install.sh" --update
  [ "$status" -eq 0 ]
  [ -f "$TMP/home/.autospec/scripts/advisor-escalate.sh" ]
  [ -f "$TMP/home/.autospec/scripts/advisor-report.sh" ]
  rm -rf "$TMP"
}
```

Note: adjust the target-dir env/flag to match what Step 1 reveals install.sh actually supports; if install.sh has no test-friendly override, assert instead that both filenames appear in the copy list (`grep -q advisor-escalate.sh install.sh`).

- [ ] **Step 3: Add the scripts to install.sh (only if it enumerates files)**

Add `advisor-escalate.sh` and `advisor-report.sh` to the explicit copy list, mirroring how `worktree-guard.sh`/`claim-guard.sh` are listed.

- [ ] **Step 4: Run the test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/advisor-install.bats`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add install.sh skills/autospec-shared/tests/unit/advisor-install.bats
git commit -m "build: ship advisor-escalate.sh + advisor-report.sh in install.sh"
```

---

### Task 5: Wire the `reviewer` gate + advisor-invocation contract into the `autospec-run` trio

**Files:**
- Modify: `skills/autospec-run/SKILL.md` (author here only)
- Regenerate: `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md`
- Modify (if new heading added): `skills/autospec-run/validate.sh` or the repo `validate.sh` named-content checks
- Regenerate: skill goldens

**Interfaces:**
- Consumes: `advisor-escalate.sh` precheck/record (Tasks 1–2).
- Produces: the `## Advisor escalation` contract section (dispatch ladder + response contract) and the `reviewer`-gate trigger prose, byte-identical across the trio.

- [ ] **Step 1: Add the contract + reviewer-gate prose to SKILL.md**

Insert a new section `## Advisor escalation` containing exactly:

```markdown
## Advisor escalation

When enabled (`AUTOSPEC_ADVISOR=on` and the gate id is in `AUTOSPEC_ADVISOR_GATES`),
a bounded hard decision may be escalated to a harness-native TIER_A **advisor**
that returns advice only.

**Protocol (every gate):**
1. Write the decision-scoped question to a temp file and the minimal relevant
   context to a second temp file. Do NOT dump the whole issue/diff.
2. `advisor-escalate.sh --phase precheck --issue <N> --repo <R> --gate <id>
   --question-file <q> --context-file <c> --json`. Non-zero exit (`7` cap, `8`
   disabled) → skip escalation and proceed exactly as if no advisor exists.
3. On `GO`, dispatch the advisor via the highest available rung:
   (1) the native advisor tool if your harness exposes it; else
   (2) a read-only TIER_A subagent (Claude `Agent(model: opus)`, OpenCode `task`
       top-tier) — preferred; else
   (3) the `cli_fallback` command from the precheck output (Codex `codex exec`;
       `claude -p`/`opencode run` are legacy). Use rung 3 only when your context
       lacks a subagent tool (e.g. a background-dispatched implementer).
   Prompt the advisor with the payload and: "Return advice only as one JSON
   object {verdict, guidance, confidence}. You have no tools and produce no
   user-facing output. guidance <= 700 tokens."
4. `advisor-escalate.sh --phase record --issue <N> --repo <R> --gate <id>
   --response-file <resp> --json`. Act on the returned verdict: `plan`/`correction`
   → apply and continue; `stop` → soft-fail (return-to-queue + comment).

**`reviewer` gate:** after the LGTM reviewer forms a verdict that is neither a
clean LGTM nor a hard BLOCK (a borderline call), run the protocol with
`--gate reviewer` to have the advisor uphold or refine the verdict before it is
issued. This complements the existing reuse-BLOCK cheap-refute pass.
```

- [ ] **Step 2: Regenerate the trio**

Run: `bash skills/autospec-shared/scripts/derive-trio.sh --in-place skills/autospec-run`
(or the repo's canonical derive command). Then verify:
Run: `diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md) <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/codex/prompt.md)`
Expected: empty (byte-identical bodies).

- [ ] **Step 3: Update validate.sh named-content checks for the new heading**

Run: `grep -n 'Advisor escalation\|named-content\|check_lockstep\|required_section' skills/autospec-run/validate.sh` (or repo `validate.sh`).
If the validator enumerates required section headings for this trio, add `## Advisor escalation` to that list so the check covers it.

- [ ] **Step 4: Regenerate goldens**

Run: `bash skills/autospec-shared/scripts/gen-skill-goldens.sh` (or repo canonical).

- [ ] **Step 5: Run validate to verify lock-step + goldens are consistent**

Run: `bash validate.sh` (repo root) or `bash skills/autospec-run/validate.sh`
Expected: exit 0.

- [ ] **Step 6: Commit (trio prose + goldens together)**

```bash
git add skills/autospec-run/SKILL.md skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md validate.sh \
        tests/ skills/**/tests/skill-goldens* 2>/dev/null
git commit -m "feat: advisor-escalation contract + reviewer gate in autospec-run trio"
```

---

### Task 6: Wire `impl-haiku` + `impl-decision` gates into the Phase 4 implementer prompt

**Files:**
- Modify: `skills/autospec-run/prompts/phase4-implementer.md` (standalone — edit directly, not a trio)

**Interfaces:**
- Consumes: the `## Advisor escalation` protocol from Task 5 (referenced, not repeated).
- Produces: gate trigger prose at the two Phase 4 decision points.

- [ ] **Step 1: Add the `impl-haiku` gate at the Expand ambiguity branch**

In the Expand step, immediately before the existing "post a clarifying comment on the issue and exit" instruction for an ambiguous contract, insert:

```markdown
> **Advisor gate `impl-haiku` (only on the `claude-haiku-cloud` profile).** Before
> taking the ambiguous-contract exit-to-queue branch, run the `## Advisor
> escalation` protocol with `--gate impl-haiku`, sending the specific ambiguity as
> the question and the relevant issue-body excerpt as context. If the advisor
> returns `plan`/`correction`, apply it and continue implementing instead of
> exiting; if it returns `stop` (or precheck was cap-reached/disabled), take the
> normal exit-to-queue path.
```

- [ ] **Step 2: Add the `impl-decision` gate at the Implement step**

In the Implement step, add:

```markdown
> **Advisor gate `impl-decision` (any profile).** When you face a design or
> architecture sub-decision you cannot reasonably resolve within your tier's
> budget, run the `## Advisor escalation` protocol with `--gate impl-decision`
> before guessing. Apply a returned `plan`/`correction`; on `stop` take the
> soft-fail path. The shared per-issue cap bounds how often this fires.
```

- [ ] **Step 3: Verify no trio drift (this file is standalone)**

Run: `grep -rl 'impl-decision' skills/autospec-run/`
Expected: only `phase4-implementer.md` (confirming it is not mirrored into codex/opencode copies).

- [ ] **Step 4: Run validate**

Run: `bash validate.sh`
Expected: exit 0 (standalone prompt file is not lock-step-gated).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/prompts/phase4-implementer.md
git commit -m "feat: impl-haiku + impl-decision advisor gates in Phase 4 implementer"
```

---

### Task 7: Wire the `retry` gate into the implementer contract

**Files:**
- Modify: `skills/autospec-run/prompts/implementer-contract.md` (standalone — edit directly)

**Interfaces:**
- Consumes: the `## Advisor escalation` protocol (Task 5).
- Produces: retry-gate prose in the Retry/review loop contract.

- [ ] **Step 1: Add the `retry` gate to the adaptive-retry step**

In `## Retry / review loop contract`, in the "Adaptive retry" item, after the sentence about re-running with accumulated corrective directives, insert:

```markdown
   On the **second or later** retry of the *same* RULE_ID, before re-running,
   run the `## Advisor escalation` protocol with `--gate retry` — send the stuck
   RULE_ID + the failing finding as the question. Inject a returned
   `plan`/`correction` as an additional corrective directive for this retry
   instead of blindly re-running the same tier; on `stop` (or cap/disabled), fall
   back to the normal retry/exhaustion behavior.
```

- [ ] **Step 2: Verify standalone (no trio drift)**

Run: `grep -rl -- '--gate retry' skills/autospec-run/`
Expected: only `implementer-contract.md`.

- [ ] **Step 3: Run validate**

Run: `bash validate.sh`
Expected: exit 0.

- [ ] **Step 4: Commit**

```bash
git add skills/autospec-run/prompts/implementer-contract.md
git commit -m "feat: retry advisor gate in implementer contract"
```

---

### Task 8: Document the env vars near the Haiku trial

**Files:**
- Modify: `examples/model-profiles.yml`

**Interfaces:**
- Consumes: nothing.
- Produces: operator-facing documentation of the advisor flags.

- [ ] **Step 1: Add a comment block after the `claude-haiku-cloud` profile**

Append:

```yaml
# ── Advisor pattern (docs/specs/2026-07-08-autospec-advisor-pattern-design.md) ──
# On-demand strong-model escalation for cheap-tier executors. Off by default.
#   AUTOSPEC_ADVISOR=on            enable the layer (default: off)
#   AUTOSPEC_ADVISOR_GATES=...     comma list of enabled gates
#                                  (default: impl-haiku; add reviewer,retry,impl-decision
#                                   only after advisor-report.sh shows promote=true)
#   AUTOSPEC_ADVISOR_MAX_USES=3    per-issue cap, shared across gates
#   AUTOSPEC_ADVISOR_MAX_CHARS=2800  guidance budget (~700 tokens)
# Rollback: AUTOSPEC_ADVISOR=off, or drop a gate from AUTOSPEC_ADVISOR_GATES.
```

- [ ] **Step 2: Verify YAML still parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('examples/model-profiles.yml'))"`
Expected: no output, exit 0.

- [ ] **Step 3: Commit**

```bash
git add examples/model-profiles.yml
git commit -m "docs: document advisor env vars in model-profiles example"
```

---

### Task 9: Full-suite gate + fresh-install / off-by-default acceptance

**Files:**
- No new source; runs the repo gates as the final acceptance.

- [ ] **Step 1: Run the shared unit suite**

Run: `bats skills/autospec-shared/tests/unit/advisor-escalate.bats skills/autospec-shared/tests/unit/advisor-report.bats skills/autospec-shared/tests/unit/advisor-install.bats`
Expected: all PASS.

- [ ] **Step 2: Run the repo validation gate**

Run: `bash validate.sh`
Expected: exit 0 (trio lock-step + goldens consistent).

- [ ] **Step 3: Off-by-default behavior check**

Run: `AUTOSPEC_ADVISOR="" bash skills/autospec-shared/scripts/advisor-escalate.sh --phase precheck --issue 1 --repo a/b --gate impl-haiku --question-file /dev/null --context-file /dev/null --json; echo "exit=$?"`
Expected: `{"decision":"DISABLED"}` and `exit=8` — confirming the layer is inert unless explicitly enabled.

- [ ] **Step 4: Commit any fixups, then final commit**

```bash
git add -A
git commit -m "test: advisor pattern acceptance gates green" || echo "nothing to commit"
```

---

## Self-Review

**Spec coverage:**
- Two-responsibility architecture (script vs contract) → Tasks 1–2 (script), Task 5 (contract). ✓
- Precheck/record two-phase interface, exit codes, fail-open → Tasks 1, 2, 9-step-3. ✓
- Path-scoped state, shared cap → Task 1 (tests for both). ✓
- Response contract (verdict enum, ≤700-token budget, strip tool/user output, unparseable→stop) → Task 2. ✓
- Capability-ordered dispatch ladder (native → subagent → CLI) → Task 5 contract prose. ✓
- Four gates (impl-haiku, impl-decision, retry, reviewer) → Tasks 5–7. ✓
- Telemetry JSONL + promotion gate + advisor-report.sh → Tasks 2, 3. ✓
- Rollout order (impl-haiku first) + flag gating + rollback → Task 1 (gate check), Task 8 (docs). ✓
- install.sh ship completeness → Task 4. ✓
- Trio derivation + validate named-content + goldens atomic commit → Task 5. ✓
- Off-by-default acceptance → Task 9. ✓

**Placeholder scan:** No TBD/TODO; every code step contains full content. Task 4 Step 1 is an inspection step (legitimate — install.sh copy mechanism is unknown until read) with both branches specified. ✓

**Type consistency:** Field names consistent across tasks — `decision`, `use_count`, `cli_fallback`, `harness`, `payload_file`, `verdict`, `guidance`, `over_budget`, `tokens_out`, `gate`, `promote`. Exit codes (`0/7/8` precheck, `0/2` record) used identically in script and tests. Gate ids (`impl-haiku`, `impl-decision`, `retry`, `reviewer`) spelled identically in script tests, contract, and prompt prose. ✓

**One refinement vs the spec:** flag/gate gating is implemented *inside* `precheck` (exit 8 = disabled) rather than left as prose the executor checks — a strictly-more-deterministic realization of the spec's env-gating intent, and it makes the gating testable. Noted here for transparency.
