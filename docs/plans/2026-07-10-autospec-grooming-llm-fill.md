# Autospec grooming LLM template-fill + canary→auto — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drain the `needs-template` dead-end by LLM-filling a valid autospec template (`codex exec`), gated by a canary→auto self-governance ratchet, extending the merged v1 grooming pipeline (PR #1736).

**Architecture:** Three new/reworked deterministic scripts (`groom-fill.sh`, `groom-reconcile.sh`, `grooming-observe.sh` rework) around one `codex exec` shell-out, wired into the existing `autonomous-promote-open-issues.sh` promoter (`needs-template` branch) and `scripts/lib/autospec-loop.sh` (Tier-1.5). The existing `grooming-govern.sh` ratchet is reused unchanged; the seed state gains *canary* behavior; graduation to `template-promote` flips grooming to auto.

**Tech Stack:** bash 3.2 (macOS-safe), `jq`, `gh`, `python3` (yaml only, existing), `bats`. codex + gh stubbed via env-override seams in tests. No DB/network mocks.

## Global Constraints

- **bash 3.2 / `set -eu` safe:** `if/then/fi` not `[ x ] && cmd`; capture `rc=$?` on its own line, never inline under `&&`; guard `grep -c`/`grep` with `|| true` where a no-match is normal; no `RETURN` traps; no `[ -f <(...) ]` (write to a real temp file first).
- **Fail-closed everywhere:** any pipeline error, codex-absent/error, validate-fail, or gh-failure must NEVER cause a promotion and must NEVER count a sample as `clean`. When in doubt: hold for human / leave unresolved.
- **`clean` is provably positive only:** closed-as-completed + ≥1 merged closing PR + no `escalate:human` + no `groom:rejected`. Everything else resolved → its non-clean outcome; never-closed → `unresolved` (outcome stays null, uncounted).
- **No DB/external mocks:** codex via `AUTOSPEC_GROOM_FILL_BIN`, gh via a `AUTOSPEC_GH_BIN`-style stub or PATH shim, sub-scripts via their existing `AUTOSPEC_*_SCRIPT`/`_BIN` seams.
- **validate gates by explicit name:** every new `.bats` suite MUST be registered in `check_grooming_contract` in `scripts/validate.sh`, ordered before `check_ship_completeness`. A suite that isn't registered never runs in validate.
- **Atomic writes:** telemetry/state rewrites use `tmp` + `mv` (never in-place edit under a concurrent reader).
- **Reused v1 interfaces (verbatim):**
  - `groom-validate.sh <body-file>` → rc0 `{"ok":true}`, rc1 `{"ok":false,"findings":[...]}`. Override linter via `AUTOSPEC_LINT_ISSUE_BIN`.
  - `grooming-config.sh --key <policy|budget.max_issues_per_cycle|budget.groom_attempts_per_issue>` → env>yaml>default, fail-closed. Env overrides: `AUTOSPEC_GROOMING_POLICY`, `AUTOSPEC_GROOMING_MAX_ISSUES`, `AUTOSPEC_GROOMING_GROOM_ATTEMPTS`.
  - `grooming-observe.sh --telemetry <jsonl>` → `{"groomed_clean_merge_rate","baseline_clean_merge_rate","samples","baseline_samples"}`; zeroed + exit 0 on missing/empty/garbled.
  - `grooming-govern.sh show | tick --observed <json> --min-samples N` → `{active:[...]}` / `{active,action,samples}`. `ORDER="eligible-promote template-promote"`, `SEED="eligible-promote"`. Unchanged by this plan.
  - Promoter helpers: `record_routed <issue> <action> <reason>`, `record_held <issue> <reason>`, `audit_comment <issue> <body>`, `ensure_label <name>`. Seams block near line 55-62 (`GROOM_LIST`/`GROOM_GOVERN`/`GROOM_CONFIG` etc.).

---

### Task 1: `grooming-config.sh` — add `budget.canary_floor` key + `.autospec/autospec.yml` default

**Files:**
- Modify: `skills/autospec-shared/scripts/grooming-config.sh` (add a `case` arm)
- Modify: `.autospec/autospec.yml` (add `canary_floor: 5` under `grooming.budget`)
- Test: `skills/autospec-shared/tests/grooming-config.bats` (extend v1 suite)

**Interfaces:**
- Consumes: v1 `resolve()` helper + `AUTOSPEC_GROOMING_*` env pattern.
- Produces: `grooming-config.sh --key budget.canary_floor` → integer (default `5`), env override `AUTOSPEC_GROOMING_CANARY_FLOOR`.

- [ ] **Step 1: Write the failing tests** — append to `grooming-config.bats`:

```bash
@test "canary_floor: default is 5 when unset" {
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE=/nonexistent \
    bash "$CONFIG_SH" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "5" ]
}

@test "canary_floor: env override wins" {
  run env AUTOSPEC_GROOMING_CANARY_FLOOR=9 bash "$CONFIG_SH" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "9" ]
}

@test "canary_floor: reads from yaml grooming.budget.canary_floor" {
  cfg="$BATS_TEST_TMPDIR/autospec.yml"
  printf 'grooming:\n  budget:\n    canary_floor: 7\n' > "$cfg"
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE="$cfg" \
    bash "$CONFIG_SH" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "7" ]
}

@test "canary_floor: non-numeric yaml falls back to default 5" {
  cfg="$BATS_TEST_TMPDIR/autospec.yml"
  printf 'grooming:\n  budget:\n    canary_floor: banana\n' > "$cfg"
  run env -u AUTOSPEC_GROOMING_CANARY_FLOOR AUTOSPEC_CONFIG_FILE="$cfg" \
    bash "$CONFIG_SH" --key budget.canary_floor
  [ "$status" -eq 0 ]
  [ "$output" = "5" ]
}
```

(Confirm the suite's setup already sets `CONFIG_SH` to the script path; if not, mirror the existing v1 `setup()`.)

- [ ] **Step 2: Run to verify fail** — `bats skills/autospec-shared/tests/grooming-config.bats` → the 4 new tests FAIL (`unknown key: budget.canary_floor`).

- [ ] **Step 3: Add the `case` arm** — in `grooming-config.sh`, immediately after the `budget.groom_attempts_per_issue)` arm:

```bash
  budget.canary_floor)
    v="$(resolve "${AUTOSPEC_GROOMING_CANARY_FLOOR:-}" "budget.canary_floor" "5")"
    case "$v" in ''|*[!0-9]*) v=5 ;; esac
    printf '%s\n' "$v"
    ;;
```

Also update the `--help` usage string and the top-of-file usage comment to list `budget.canary_floor`.

- [ ] **Step 4: Add the yaml default** — in `.autospec/autospec.yml`, under `grooming.budget`:

```yaml
grooming:
  policy: auto
  budget:
    max_issues_per_cycle: 5
    groom_attempts_per_issue: 2
    canary_floor: 5
```

- [ ] **Step 5: Run to verify pass** — `bats skills/autospec-shared/tests/grooming-config.bats` → all pass.

- [ ] **Step 6: Commit** — `git add skills/autospec-shared/scripts/grooming-config.sh skills/autospec-shared/tests/grooming-config.bats .autospec/autospec.yml && git commit -m "feat(grooming): budget.canary_floor config key (default 5)"`

---

### Task 2: `groom-fill.sh` — codex exec template-fill + adaptive retry

**Files:**
- Create: `scripts/groom-fill.sh`
- Test: `tests/autospec/groom-fill.bats` (new — same dir as v1's `tests/autospec/groom-validate.bats`)

**Interfaces:**
- Consumes: `groom-validate.sh <body-file>` (v1); `grooming-config.sh --key budget.groom_attempts_per_issue`.
- Produces: `groom-fill.sh --issue <n> --repo <owner/name> [--attempts N] [--title T --body B]` →
  - success: `{"ok":true,"body":"<filled template>"}` (rc 0)
  - failure: `{"ok":false,"reason":"codex-absent|codex-error|validate-failed|attempts-exhausted"}` (rc 1)
- Seams (test injection):
  - `AUTOSPEC_GROOM_FILL_BIN` — the codex binary/stub (default `codex`). Invoked as `"$BIN" exec` reading a prompt on stdin, writing the filled body on stdout.
  - `AUTOSPEC_GROOM_VALIDATE_BIN` — default `<script_dir>/groom-validate.sh`.
  - `AUTOSPEC_GH_BIN` — default `gh` (used only to fetch title/body when `--title/--body` not supplied).

- [ ] **Step 1: Write the failing tests** — `scripts/tests/groom-fill.bats`:

```bash
setup() {
  FILL_SH="${BATS_TEST_DIRNAME}/../../scripts/groom-fill.sh"   # tests/autospec/ → repo/scripts/
  BIN_DIR="$BATS_TEST_TMPDIR/bin"; mkdir -p "$BIN_DIR"
  # default: codex stub echoes a canned "filled" body
  cat > "$BIN_DIR/codex-ok" <<'SH'
#!/usr/bin/env bash
# ignore "exec" subcommand + args, read prompt from stdin, emit a body
cat >/dev/null
printf '## Summary\nFilled template body.\n## Acceptance\n- done\n'
SH
  chmod +x "$BIN_DIR/codex-ok"
  # validate stub: ok
  cat > "$BIN_DIR/validate-ok" <<'SH'
#!/usr/bin/env bash
printf '{"ok":true}\n'; exit 0
SH
  chmod +x "$BIN_DIR/validate-ok"
}

@test "codex-absent → ok:false reason codex-absent, holds" {
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/does-not-exist" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "false" ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "codex-absent" ]
}

@test "codex-ok + validate-ok → ok:true with body" {
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-ok" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --title T --body B
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "true" ]
  printf '%s' "$output" | jq -e '.body | test("Filled template body")' >/dev/null
}

@test "validate fails then succeeds within attempts → ok:true" {
  # codex emits attempt-numbered body; validate stub fails on attempt 1, ok on 2
  cat > "$BIN_DIR/codex-count" <<'SH'
#!/usr/bin/env bash
cat >/dev/null
n=$(cat "$COUNTER" 2>/dev/null || echo 0); n=$((n+1)); echo "$n" > "$COUNTER"
printf 'BODY attempt %s\n' "$n"
SH
  chmod +x "$BIN_DIR/codex-count"
  cat > "$BIN_DIR/validate-2nd" <<'SH'
#!/usr/bin/env bash
grep -q 'attempt 1' "$1" && { printf '{"ok":false,"findings":["missing Summary"]}\n'; exit 1; }
printf '{"ok":true}\n'; exit 0
SH
  chmod +x "$BIN_DIR/validate-2nd"
  run env COUNTER="$BATS_TEST_TMPDIR/c" \
          AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-count" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-2nd" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 3 --title T --body B
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "true" ]
}

@test "attempts exhausted → ok:false attempts-exhausted" {
  cat > "$BIN_DIR/validate-no" <<'SH'
#!/usr/bin/env bash
printf '{"ok":false,"findings":["nope"]}\n'; exit 1
SH
  chmod +x "$BIN_DIR/validate-no"
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-ok" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-no" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 2 --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "attempts-exhausted" ]
}

@test "codex nonzero exit → ok:false codex-error" {
  cat > "$BIN_DIR/codex-fail" <<'SH'
#!/usr/bin/env bash
cat >/dev/null; exit 7
SH
  chmod +x "$BIN_DIR/codex-fail"
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-fail" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 2 --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "codex-error" ]
}
```

- [ ] **Step 2: Run to verify fail** — `bats tests/autospec/groom-fill.bats` → all FAIL (script absent).

- [ ] **Step 3: Implement `scripts/groom-fill.sh`:**

```bash
#!/usr/bin/env bash
# scripts/groom-fill.sh — LLM template-fill for a needs-template issue.
#
# Shells out to `codex exec` (override via AUTOSPEC_GROOM_FILL_BIN) with the raw
# issue + the autospec template contract, then validates the result with
# groom-validate.sh (override via AUTOSPEC_GROOM_VALIDATE_BIN), retrying up to
# --attempts with the linter findings fed back as directives.
#
# Fail-closed: codex absent/error or validation never passing → ok:false; the
# caller MUST hold the issue for a human (never promote an unvalidated body).
#
# Usage:
#   groom-fill.sh --issue N --repo O/R [--attempts K] [--title T --body B]
# Output:
#   rc 0: {"ok":true,"body":"<filled template>"}
#   rc 1: {"ok":false,"reason":"codex-absent|codex-error|validate-failed|attempts-exhausted"}
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FILL_BIN="${AUTOSPEC_GROOM_FILL_BIN:-codex}"
VALIDATE_BIN="${AUTOSPEC_GROOM_VALIDATE_BIN:-$SCRIPT_DIR/groom-validate.sh}"
GH_BIN="${AUTOSPEC_GH_BIN:-gh}"

ISSUE="" REPO="" ATTEMPTS="" TITLE="" BODY="" HAVE_BODY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --issue) ISSUE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --attempts) ATTEMPTS="${2:-}"; shift 2 ;;
    --title) TITLE="${2:-}"; shift 2 ;;
    --body) BODY="${2:-}"; HAVE_BODY=1; shift 2 ;;
    --help|-h) printf 'Usage: groom-fill.sh --issue N --repo O/R [--attempts K] [--title T --body B]\n'; exit 0 ;;
    *) printf 'groom-fill.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[ -n "$ISSUE" ] || { printf 'groom-fill.sh: --issue required\n' >&2; exit 2; }
[ -n "$REPO" ] || { printf 'groom-fill.sh: --repo required\n' >&2; exit 2; }

fail() { jq -cn --arg r "$1" '{ok:false,reason:$r}'; exit 1; }

# codex binary must be resolvable (absolute path OR on PATH).
if ! command -v "$FILL_BIN" >/dev/null 2>&1 && [ ! -x "$FILL_BIN" ]; then
  fail "codex-absent"
fi

# Resolve attempts from arg → config → default 2.
if [ -z "$ATTEMPTS" ]; then
  if [ -f "$SCRIPT_DIR/../skills/autospec-shared/scripts/grooming-config.sh" ]; then
    ATTEMPTS="$(bash "$SCRIPT_DIR/../skills/autospec-shared/scripts/grooming-config.sh" \
                  --key budget.groom_attempts_per_issue 2>/dev/null || printf '2')"
  fi
fi
case "$ATTEMPTS" in ''|*[!0-9]*) ATTEMPTS=2 ;; esac

# Fetch title/body if not supplied.
if [ "$HAVE_BODY" -ne 1 ]; then
  raw="$("$GH_BIN" issue view "$ISSUE" --repo "$REPO" --json title,body 2>/dev/null || printf '')"
  if [ -n "$raw" ]; then
    TITLE="$(printf '%s' "$raw" | jq -r '.title // ""')"
    BODY="$(printf '%s' "$raw" | jq -r '.body // ""')"
  fi
fi

tmp_body="$(mktemp "${TMPDIR:-/tmp}/groom-fill.XXXXXX")"
trap 'rm -f "$tmp_body"' EXIT

directives=""
attempt=1
while [ "$attempt" -le "$ATTEMPTS" ]; do
  prompt="$(printf 'Fill the autospec issue template for the following raw issue.\nReturn ONLY the filled markdown body.\n\nTITLE: %s\n\nBODY:\n%s\n%s\n' \
            "$TITLE" "$BODY" "${directives:+PREVIOUS VALIDATION FINDINGS TO FIX:\n$directives}")"

  set +e
  filled="$(printf '%s' "$prompt" | "$FILL_BIN" exec 2>/dev/null)"
  rc=$?
  set -e
  if [ "$rc" -ne 0 ]; then
    fail "codex-error"
  fi
  if [ -z "$filled" ]; then
    directives="codex returned empty body"
    attempt=$((attempt + 1))
    continue
  fi

  printf '%s' "$filled" > "$tmp_body"
  set +e
  vout="$("$VALIDATE_BIN" "$tmp_body" 2>/dev/null)"
  vrc=$?
  set -e
  if [ "$vrc" -eq 0 ]; then
    jq -cn --arg b "$filled" '{ok:true,body:$b}'
    exit 0
  fi
  directives="$(printf '%s' "$vout" | jq -r '(.findings // []) | join("; ")' 2>/dev/null || printf '')"
  attempt=$((attempt + 1))
done

fail "attempts-exhausted"
```

- [ ] **Step 4: Run to verify pass** — `bats tests/autospec/groom-fill.bats` → all pass.

- [ ] **Step 5: Register in validate** — add `tests/autospec/groom-fill.bats` to `check_grooming_contract` in `scripts/validate.sh` (mirror the existing per-suite `"tests/autospec/…"` lines, and bump the "seven grooming bats suites" wording). Run `bash scripts/validate.sh 2>&1 | grep -i groom-fill` to confirm it appears in the run.

- [ ] **Step 6: Commit** — `git add scripts/groom-fill.sh tests/autospec/groom-fill.bats scripts/validate.sh && git commit -m "feat(grooming): groom-fill.sh — codex exec template-fill + adaptive retry"`

---

### Task 3: `groom-reconcile.sh` — stamp real outcomes into the telemetry log from gh

**Files:**
- Create: `scripts/groom-reconcile.sh`
- Test: `tests/autospec/groom-reconcile.bats` (new — same dir as v1 grooming suites)

**Interfaces:**
- Consumes: the telemetry JSONL (records with `issue`, `template_groomed`, `outcome`).
- Produces: `groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>` — rewrites the log in place (atomic), stamping `closing_pr` + `outcome` for unresolved+closed records. rc 0 always (never blocks the sweep). Seam: `AUTOSPEC_GH_BIN` (default `gh`), invoked `"$GH_BIN" issue view <n> --repo <r> --json state,stateReason,closedByPullRequestsReferences,labels`.
- Outcome mapping (fail-closed toward non-clean):
  - `escalate` — labels include `escalate:human`.
  - else `rejected` — labels include `groom:rejected`, OR state CLOSED with stateReason != COMPLETED, OR CLOSED with no merged closing PR.
  - else `clean` — state CLOSED, stateReason COMPLETED, ≥1 `closedByPullRequestsReferences` entry (proof of a closing PR), no `escalate:human`, no `groom:rejected`.
  - else (state OPEN) → leave `outcome:null` (`unresolved`; a record open again after a prior promote is simply re-observed next cycle).

- [ ] **Step 1: Write the failing tests** — `scripts/tests/groom-reconcile.bats`:

```bash
setup() {
  RECON_SH="${BATS_TEST_DIRNAME}/../../scripts/groom-reconcile.sh"   # tests/autospec/ → repo/scripts/
  BIN_DIR="$BATS_TEST_TMPDIR/bin"; mkdir -p "$BIN_DIR"
  TELE="$BATS_TEST_TMPDIR/tele.jsonl"
}
# a gh stub that returns a fixed json per issue number from files issue-<n>.json
mk_gh() {
  cat > "$BIN_DIR/gh" <<'SH'
#!/usr/bin/env bash
# args: issue view N --repo R --json ...
num=""
while [ $# -gt 0 ]; do case "$1" in view) num="$2"; shift 2;; *) shift;; esac; done
f="$GH_FIXTURE_DIR/issue-$num.json"
[ -f "$f" ] && cat "$f" || { echo '{}'; }
SH
  chmod +x "$BIN_DIR/gh"
}

@test "unresolved + closed-completed + merged PR → clean" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":42}],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-10.json"
  printf '{"issue":10,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "clean" ]
  [ "$(jq -r '.closing_pr' "$TELE")" = "42" ]
}

@test "escalate:human label → escalate (even if closed clean-looking)" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":9}],"labels":[{"name":"escalate:human"}]}\n' > "$BATS_TEST_TMPDIR/fx/issue-11.json"
  printf '{"issue":11,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "escalate" ]
}

@test "closed not-planned → rejected" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"NOT_PLANNED","closedByPullRequestsReferences":[],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-12.json"
  printf '{"issue":12,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "rejected" ]
}

@test "groom:rejected label → rejected" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":5}],"labels":[{"name":"groom:rejected"}]}\n' > "$BATS_TEST_TMPDIR/fx/issue-13.json"
  printf '{"issue":13,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "rejected" ]
}

@test "still open → stays unresolved (null)" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"OPEN","stateReason":null,"closedByPullRequestsReferences":[],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-14.json"
  printf '{"issue":14,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "null" ]
}

@test "already-resolved record untouched (no gh call)" {
  # gh stub that fails loudly if called
  cat > "$BIN_DIR/gh-boom" <<'SH'
#!/usr/bin/env bash
echo "SHOULD NOT CALL" >&2; exit 99
SH
  chmod +x "$BIN_DIR/gh-boom"
  printf '{"issue":15,"template_groomed":true,"outcome":"clean","closing_pr":1}\n' > "$TELE"
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh-boom" bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "clean" ]
}

@test "gh failure leaves outcome null (fail-closed)" {
  cat > "$BIN_DIR/gh-fail" <<'SH'
#!/usr/bin/env bash
exit 1
SH
  chmod +x "$BIN_DIR/gh-fail"
  printf '{"issue":16,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh-fail" bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "null" ]
}

@test "missing telemetry file → rc 0, no crash" {
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh" bash "$RECON_SH" --telemetry /nonexistent --repo o/r
  [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run to verify fail** — `bats tests/autospec/groom-reconcile.bats` → all FAIL.

- [ ] **Step 3: Implement `scripts/groom-reconcile.sh`:**

```bash
#!/usr/bin/env bash
# scripts/groom-reconcile.sh — stamp real clean-merge outcomes into the grooming
# telemetry log from GitHub. For each record with outcome==null whose issue is
# now closed, query gh and set closing_pr + outcome. Bounded to unresolved
# records; a gh failure leaves the record unresolved (never counted clean).
#
# `clean` is provably positive ONLY: CLOSED + stateReason COMPLETED + >=1 closing
# PR + no escalate:human + no groom:rejected. escalate/rejected take precedence.
#
# Usage: groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>
# Exit: always 0 (never blocks the sweep).
set -eu

GH_BIN="${AUTOSPEC_GH_BIN:-gh}"
TELE="" REPO=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --help|-h) printf 'Usage: groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>\n'; exit 0 ;;
    *) printf 'groom-reconcile.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[ -n "$TELE" ] && [ -f "$TELE" ] || exit 0
[ -n "$REPO" ] || exit 0

# Classify one gh JSON blob → outcome string, or empty to leave unresolved.
classify() {
  printf '%s' "$1" | jq -r '
    def has_label($n): ((.labels // []) | map(.name) | index($n)) != null;
    if .state != "CLOSED" then ""
    elif has_label("escalate:human") then "escalate"
    elif has_label("groom:rejected") then "rejected"
    elif (.stateReason // "") != "COMPLETED" then "rejected"
    elif ((.closedByPullRequestsReferences // []) | length) == 0 then "rejected"
    else "clean" end'
}
closing_pr_of() {
  printf '%s' "$1" | jq -r '(.closedByPullRequestsReferences // [] | .[0].number) // "null"'
}

tmp="$(mktemp "${TELE}.XXXXXX")"
# Process line-by-line; malformed lines are passed through unchanged.
while IFS= read -r line || [ -n "$line" ]; do
  if [ -z "$line" ]; then continue; fi
  # Only touch well-formed records with outcome==null.
  need="$(printf '%s' "$line" | jq -r 'if (type=="object" and (.outcome==null) and (.issue!=null)) then .issue else "skip" end' 2>/dev/null || printf 'skip')"
  if [ "$need" = "skip" ] || [ -z "$need" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue
  fi
  set +e
  blob="$("$GH_BIN" issue view "$need" --repo "$REPO" \
            --json state,stateReason,closedByPullRequestsReferences,labels 2>/dev/null)"
  ghrc=$?
  set -e
  if [ "$ghrc" -ne 0 ] || [ -z "$blob" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # fail-closed: unresolved
  fi
  oc="$(classify "$blob")"
  if [ -z "$oc" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # still open → unresolved
  fi
  pr="$(closing_pr_of "$blob")"
  printf '%s' "$line" | jq -c --arg oc "$oc" --argjson pr "$pr" \
    '.outcome=$oc | .closing_pr=$pr' >> "$tmp"
done < "$TELE"
mv "$tmp" "$TELE"
exit 0
```

- [ ] **Step 4: Run to verify pass** — `bats tests/autospec/groom-reconcile.bats` → all pass.

- [ ] **Step 5: Register in validate** — add `tests/autospec/groom-reconcile.bats` to `check_grooming_contract`. Confirm via `bash scripts/validate.sh 2>&1 | grep -i groom-reconcile`.

- [ ] **Step 6: Commit** — `git add scripts/groom-reconcile.sh tests/autospec/groom-reconcile.bats scripts/validate.sh && git commit -m "feat(grooming): groom-reconcile.sh — stamp real outcomes from gh (fail-closed)"`

---

### Task 4: `grooming-observe.sh` rework — template-groom partition + reconciled outcome

**Files:**
- Modify: `skills/autospec-shared/scripts/grooming-observe.sh`
- Test: `skills/autospec-shared/tests/unit/grooming-observe.bats` (extend v1)

**Interfaces:**
- Consumes: telemetry JSONL now carrying `template_groomed` + reconciled `outcome`.
- Produces: same envelope, but `samples`/`groomed_clean_merge_rate` now scoped to **resolved template-groom** records, `baseline_*` to **resolved non-template-groom** records.

**Semantics change (document in header):**
- A record is **template-groom** when `template_groomed == true`.
- A record is **resolved** when `outcome != null`.
- `samples` = resolved template-groom records; `groomed_clean_merge_rate` over them.
- `baseline_samples` = resolved non-template-groom records; `baseline_clean_merge_rate` over them.
- `is_clean`: `outcome == "clean"` when `outcome` present; **back-compat** — when `outcome` is absent (pre-enhancement record), fall back to v1's `(.reverted != true) and (.reopened != true) and no escalate:human`.

- [ ] **Step 1: Write the failing tests** — append to `grooming-observe.bats`:

```bash
@test "samples counts only resolved template-groom records" {
  t="$BATS_TEST_TMPDIR/t.jsonl"
  {
    printf '{"issue":1,"template_groomed":true,"outcome":"clean"}\n'
    printf '{"issue":2,"template_groomed":true,"outcome":"rejected"}\n'
    printf '{"issue":3,"template_groomed":true,"outcome":null}\n'   # unresolved: excluded
    printf '{"issue":4,"template_groomed":false,"outcome":"clean"}\n' # baseline
  } > "$t"
  run bash "$OBSERVE_SH" --telemetry "$t"
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .samples)" = "2" ]
  [ "$(printf '%s' "$output" | jq -r .groomed_clean_merge_rate)" = "0.5" ]
  [ "$(printf '%s' "$output" | jq -r .baseline_samples)" = "1" ]
  [ "$(printf '%s' "$output" | jq -r .baseline_clean_merge_rate)" = "1" ]
}

@test "back-compat: pre-enhancement record without outcome uses reverted/reopened" {
  t="$BATS_TEST_TMPDIR/t.jsonl"
  printf '{"issue":1,"template_groomed":true,"reverted":false,"reopened":false}\n' > "$t"
  # No outcome field → treated as resolved-clean via legacy fallback.
  run bash "$OBSERVE_SH" --telemetry "$t"
  [ "$(printf '%s' "$output" | jq -r .samples)" = "1" ]
  [ "$(printf '%s' "$output" | jq -r .groomed_clean_merge_rate)" = "1" ]
}
```

(Match the v1 suite's `OBSERVE_SH` setup var.)

- [ ] **Step 2: Run to verify fail** — the new tests FAIL (v1 counts by `source`/`groomed`, includes unresolved).

- [ ] **Step 3: Rework the jq aggregate** in `grooming-observe.sh` — replace the `def is_groomed`/`def is_clean` block and the partition:

```bash
out="$(jq -R 'fromjson? // empty' "$TELEMETRY" 2>/dev/null | jq -s '
  def is_template_groom: (.template_groomed == true);
  def is_resolved: (has("outcome") and (.outcome != null))
                   or ((has("outcome") | not) and (has("reverted") or has("reopened")));
  def is_clean:
    if (has("outcome") and (.outcome != null)) then (.outcome == "clean")
    else
      ((.reverted != true) and (.reopened != true)
       and ((.labels // []) | index("escalate:human") | not)
       and (.verdict != "escalate:human"))
    end;
  ([.[] | select(is_resolved)]) as $r
  | ([$r[] | select(is_template_groom)]) as $g
  | ([$r[] | select(is_template_groom | not)]) as $b
  | ($g | length) as $gn | ($b | length) as $bn
  | ([$g[] | select(is_clean)] | length) as $gc
  | ([$b[] | select(is_clean)] | length) as $bc
  | {
      groomed_clean_merge_rate:  (if $gn > 0 then ($gc / $gn) else 0 end),
      baseline_clean_merge_rate: (if $bn > 0 then ($bc / $bn) else 0 end),
      samples: $gn,
      baseline_samples: $bn
    }
' 2>/dev/null)" || true
```

Update the header comment block to describe the template-groom/baseline partition and the reconciled-outcome semantics.

- [ ] **Step 4: Run to verify pass** — full `bats grooming-observe.bats` (v1 + new) → all pass. If a v1 test asserted the old `source`-based counting, update it to the new template-groom semantics (the ratchet now governs template-grooms, not eligible-promotes) and note the change in the commit body.

- [ ] **Step 5: Commit** — `git add skills/autospec-shared/scripts/grooming-observe.sh skills/autospec-shared/tests/unit/grooming-observe.bats && git commit -m "feat(grooming): observe measures template-groom clean rate via reconciled outcome"`

---

### Task 5: Promoter `needs-template` branch — canary (seed) vs auto (graduated) groom

**Files:**
- Modify: `scripts/autonomous-promote-open-issues.sh` (seams block + `needs-template)` case + skip-already-proposed)
- Test: `tests/autospec/autonomous-promote-open-issues.bats` (extend v1) — or wherever v1's promoter suite lives

**Interfaces:**
- Consumes: `groom-fill.sh` via new seam `GROOM_FILL="${AUTOSPEC_GROOM_FILL_SCRIPT:-$SCRIPT_DIR/groom-fill.sh}"`; `grooming-govern.sh show` (existing) for active set.
- Produces: `routed[]` entries with `action ∈ "groom-canary" | "groom-auto"` (the loop derives `template_groomed:true` + `canary` from these). Labels: canary → `groom:proposed` (+ removes `needs-autospec-template`); auto → `auto-implement` (+ removes `needs-autospec-template`).

- [ ] **Step 1: Write the failing tests** — append to the promoter suite. Stub `groom-fill.sh` (ok/fail) and `grooming-govern.sh show` via seams; stub `gh` to a no-op recorder:

```bash
@test "needs-template + seed state → canary: groom:proposed, no auto-implement" {
  # eligibility stub → needs-template; govern show → seed (no template-promote);
  # fill stub → ok body; gh recorder captures --add-label
  setup_promoter_stubs decision=needs-template govern='["eligible-promote"]' fill=ok
  run run_promoter --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label groom:proposed' "$GH_LOG"
  grep -q 'remove-label needs-autospec-template' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.routed[] | select(.action=="groom-canary")' >/dev/null
}

@test "needs-template + template-promote active → auto: auto-implement, no groom:proposed" {
  setup_promoter_stubs decision=needs-template govern='["eligible-promote","template-promote"]' fill=ok
  run run_promoter --apply
  grep -q 'add-label auto-implement' "$GH_LOG"
  grep -q 'remove-label needs-autospec-template' "$GH_LOG"
  ! grep -q 'add-label groom:proposed' "$GH_LOG"
  printf '%s' "$output" | jq -e '.routed[] | select(.action=="groom-auto")' >/dev/null
}

@test "needs-template + fill fails → hold:needs-human (no promote)" {
  setup_promoter_stubs decision=needs-template govern='["eligible-promote","template-promote"]' fill=fail
  run run_promoter --apply
  grep -q 'add-label hold:needs-human' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.held[] | select(.reason | test("fill-"))' >/dev/null
}

@test "already groom:proposed candidate is skipped (no re-fill)" {
  setup_promoter_stubs decision=needs-template govern='["eligible-promote"]' fill=ok \
    candidate_labels='["needs-autospec-template","groom:proposed"]'
  run run_promoter --apply
  ! grep -q 'add-label groom:proposed' "$GH_LOG"   # not re-proposed
}
```

Implement `setup_promoter_stubs`/`run_promoter`/`$GH_LOG` in the suite's helpers, following v1's existing stub harness (v1 already stubs `GROOM_LIST`/`GROOM_ELIGIBILITY`/`GROOM_GOVERN` via `AUTOSPEC_GROOM_*_SCRIPT` and `gh` via PATH). The candidate list stub must include the issue's current labels so the skip check can read them.

- [ ] **Step 2: Run to verify fail** — new tests FAIL (current branch applies `route:groom`/holds, no fill).

- [ ] **Step 3: Add the seam** — in the seams block (~line 62):

```bash
GROOM_FILL="${AUTOSPEC_GROOM_FILL_SCRIPT:-$SCRIPT_DIR/groom-fill.sh}"
```

- [ ] **Step 4: Add a skip for already-proposed/rejected** — where the promoter iterates candidates in the apply path, before deciding, skip issues whose labels include `groom:proposed` or `groom:rejected` (read from the candidate's label list that `list-groomable.sh` returns, or a `gh issue view --json labels` when absent):

```bash
case " $cand_labels " in
  *" groom:proposed "*|*" groom:rejected "*)
    record_skip "$num" "already-groomed"; continue ;;
esac
```

- [ ] **Step 5: Replace the `needs-template)` case** with canary/auto:

```bash
        needs-template)
            active="$(bash "$GROOM_GOVERN" show 2>/dev/null | jq -r '.active // [] | join(" ")' 2>/dev/null || printf '')"
            fill_out="$(bash "$GROOM_FILL" --issue "$num" --repo "$repo" 2>/dev/null || printf '')"
            fill_ok="$(printf '%s' "$fill_out" | jq -r '.ok // false' 2>/dev/null || printf 'false')"
            if [ "$fill_ok" != "true" ]; then
                freason="$(printf '%s' "$fill_out" | jq -r '.reason // "fill-error"' 2>/dev/null || printf 'fill-error')"
                ensure_label "hold:needs-human"
                gh issue edit "$num" --repo "$repo" --add-label "hold:needs-human" >/dev/null 2>&1 || true
                audit_comment "$num" "grooming: codex template-fill unavailable/failed (${freason}) — held for human (${ereason})."
                record_held "$num" "needs-template:fill-${freason}"
            else
                fbody="$(printf '%s' "$fill_out" | jq -r '.body')"
                bf="$(mktemp "${TMPDIR:-/tmp}/groom-body.XXXXXX")"
                printf '%s' "$fbody" > "$bf"
                case " $active " in
                    *" template-promote "*)
                        ensure_label "auto-implement"
                        gh issue edit "$num" --repo "$repo" --body-file "$bf" \
                            --add-label "auto-implement" \
                            --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
                        audit_comment "$num" "grooming: auto-template-groomed — template-promote gate active (${ereason})."
                        record_routed "$num" "groom-auto" "needs-template:${ereason}"
                        ;;
                    *)
                        ensure_label "groom:proposed"
                        gh issue edit "$num" --repo "$repo" --body-file "$bf" \
                            --add-label "groom:proposed" \
                            --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
                        audit_comment "$num" "grooming: template drafted for human approval (canary) — approve by adding auto-implement, reject with groom:rejected (${ereason})."
                        record_routed "$num" "groom-canary" "needs-template:${ereason}"
                        ;;
                esac
                rm -f "$bf"
            fi
            ;;
```

Update the top-of-file pipeline doc comment (`needs-template → …`) to describe canary-vs-auto instead of the old route:groom-or-hold.

- [ ] **Step 6: Run to verify pass** — full promoter suite → all pass (v1 + new).

- [ ] **Step 7: Register/confirm in validate** — the promoter suite (`tests/autospec/autonomous-promote-open-issues.bats`) is already in `check_grooming_contract`; confirm it still runs. Commit — `git add scripts/autonomous-promote-open-issues.sh tests/autospec/autonomous-promote-open-issues.bats && git commit -m "feat(grooming): needs-template canary(seed)/auto(graduated) template-fill routing"`

---

### Task 6: Loop wiring — reconcile pass + template-groom telemetry + canary_floor min-samples

**Files:**
- Modify: `scripts/lib/autospec-loop.sh` (the Tier-1.5 telemetry/govern block, ~lines 1188-1288)
- Test: `tests/autospec/test_loop_grooming.bats` (new) — exercise the block via the loop's existing test seams

**Interfaces:**
- Consumes: promoter envelope `.promoted[]` (eligible → `template_groomed:false`) and `.routed[]` with `action ∈ groom-canary|groom-auto` (→ `template_groomed:true`, `canary = action=="groom-canary"`); `grooming-config.sh --key budget.canary_floor`; `groom-reconcile.sh`.
- Produces: telemetry records `{ts,issue,source:"grooming",template_groomed,canary?,closing_pr:null,outcome:null}`; a pre-observe reconcile pass; govern tick with `--min-samples <canary_floor>`.

- [ ] **Step 1: Write the failing test** — `tests/autospec/test_loop_grooming.bats`. Drive the block with a stub promoter output containing both `promoted` and `routed`, stub `groom-reconcile.sh`/`grooming-observe.sh`/`grooming-govern.sh` via their `AUTOSPEC_GROOMING_*_BIN` seams, and assert:
  - one telemetry line per promoted issue with `template_groomed:false`;
  - one line per routed groom with `template_groomed:true` and correct `canary`;
  - `groom-reconcile.sh` invoked before `grooming-observe.sh` (assert via ordered marker file the stubs append to);
  - govern tick receives `--min-samples` equal to the configured `canary_floor` (assert via the govern stub echoing its args to a log).

(Reuse the loop's existing bats harness for invoking a single conductor action; follow `tests/autospec/test_conductor_wiring.bats` for how it stubs `_sdir` scripts and captures `_promote_out`.)

- [ ] **Step 2: Run to verify fail** — new suite FAILS (loop emits only `.promoted`, hardcodes fields, no reconcile, min-samples 20).

- [ ] **Step 3: Add the reconcile call** — in the loop block, immediately before the observe/govern tick (and before the telemetry append is read by observe), add a reconcile pass resolving the script the same lazy way as observe/govern, guarded to `policy != off` and `_dry != 1`:

```bash
# Reconcile outcomes into the telemetry log BEFORE observing (stamps
# closing_pr + outcome for unresolved+closed records; fail-closed).
local _groom_reconcile_sh="${AUTOSPEC_GROOMING_RECONCILE_BIN:-}"
if [ -z "$_groom_reconcile_sh" ]; then
    if [ -f "${_sdir}/groom-reconcile.sh" ]; then
        _groom_reconcile_sh="${_sdir}/groom-reconcile.sh"
    elif [ -f "${_sdir}/../scripts/groom-reconcile.sh" ]; then
        _groom_reconcile_sh="${_sdir}/../scripts/groom-reconcile.sh"
    fi
fi
if [ -n "$_groom_reconcile_sh" ] && [ -f "$_groom_telemetry" ]; then
    bash "$_groom_reconcile_sh" --telemetry "$_groom_telemetry" --repo "$_repo" >/dev/null 2>&1 || true
fi
```

(Place this after `_groom_telemetry` is defined; confirm `$_repo` is the conductor's repo var in scope — otherwise resolve it as the block already does for the promoter.)

- [ ] **Step 4: Emit telemetry for eligible promotes AND routed grooms** — replace the promoted-only emit loop. For each `.promoted[]` issue emit `template_groomed:false`; for each `.routed[]` entry whose `action` starts with `groom-`, emit `template_groomed:true` + `canary:(action=="groom-canary")`. All records add `closing_pr:null, outcome:null`:

```bash
# eligible promotes → baseline population (template_groomed:false)
#   (existing loop, add fields)
jq -cn --argjson issue "$_gnum" --arg ts "$_groom_ts" \
  '{ts:$ts, issue:$issue, source:"grooming", template_groomed:false,
    closing_pr:null, outcome:null}' >> "$_groom_telemetry" 2>/dev/null || true

# template grooms → graduation sample (template_groomed:true)
local _groom_routed_json
_groom_routed_json="$(printf '%s' "$_promote_out" | jq -c '[.routed[]? | select(.action|type=="string" and startswith("groom-"))]' 2>/dev/null || printf '[]')"
local _groom_routed_count
_groom_routed_count="$(printf '%s' "$_groom_routed_json" | jq 'length' 2>/dev/null || printf '0')"
case "$_groom_routed_count" in ''|*[!0-9]*) _groom_routed_count=0 ;; esac
local _rj=0
while [ "$_rj" -lt "$_groom_routed_count" ]; do
    local _rnum _ract
    _rnum="$(printf '%s' "$_groom_routed_json" | jq -r ".[$_rj].issue" 2>/dev/null || printf '')"
    _ract="$(printf '%s' "$_groom_routed_json" | jq -r ".[$_rj].action" 2>/dev/null || printf '')"
    if [ -n "$_rnum" ]; then
        local _canary=false
        [ "$_ract" = "groom-canary" ] && _canary=true
        jq -cn --argjson issue "$_rnum" --arg ts "$_groom_ts" --argjson canary "$_canary" \
          '{ts:$ts, issue:$issue, source:"grooming", template_groomed:true,
            canary:$canary, closing_pr:null, outcome:null}' >> "$_groom_telemetry" 2>/dev/null || true
    fi
    _rj=$((_rj + 1))
done
```

(Keep the `mkdir -p "$(dirname ...)"` + `_groom_ts` setup; guard the whole emit so it runs when either promoted or routed grooms exist.)

- [ ] **Step 5: Drive the tick with canary_floor** — replace the hardcoded `_groom_min_samples="${AUTOSPEC_GROOMING_MIN_SAMPLES:-20}"` with the configured `canary_floor` (env still overrides):

```bash
local _groom_min_samples="${AUTOSPEC_GROOMING_MIN_SAMPLES:-}"
if [ -z "$_groom_min_samples" ] && [ -n "$_groom_config_sh" ]; then
    _groom_min_samples="$(bash "$_groom_config_sh" --key budget.canary_floor 2>/dev/null || printf '5')"
fi
case "$_groom_min_samples" in ''|*[!0-9]*) _groom_min_samples=5 ;; esac
```

Update the block's explanatory comment: it no longer says outcomes are "enriched elsewhere / emitted false here" — reconcile stamps them; the prose route:groom contract paragraph is replaced by "grooming template-fill is performed deterministically by the promoter via groom-fill.sh (codex exec)."

- [ ] **Step 6: Run to verify pass** — `bats tests/autospec/test_loop_grooming.bats` → all pass.

- [ ] **Step 7: Register in validate** — add `tests/autospec/test_loop_grooming.bats` to `check_grooming_contract`. Confirm via `bash scripts/validate.sh 2>&1 | grep -i test_loop_grooming`.

- [ ] **Step 8: Commit** — `git add scripts/lib/autospec-loop.sh tests/autospec/test_loop_grooming.bats scripts/validate.sh && git commit -m "feat(grooming): loop reconcile pass + template-groom telemetry + canary_floor gate"`

---

### Task 7: End-to-end grooming contract + docs

**Files:**
- Modify: `scripts/validate.sh` (`check_grooming_contract` — confirm all suites registered, ordered before `check_ship_completeness`)
- Modify: any `docs/` grooming reference if v1 added one (describe the canary→auto lifecycle + `groom:proposed`/`groom:rejected` human signals)
- Test: run the full grooming contract inside validate

- [ ] **Step 1: Audit registration** — grep `check_grooming_contract` and confirm it names all suites. v1 (7, verbatim paths): `tests/autospec/list-groomable.bats`, `tests/autospec/promote-eligibility.bats`, `tests/autospec/groom-validate.bats`, `tests/autospec/autonomous-promote-open-issues.bats`, `skills/autospec-shared/tests/grooming-config.bats`, `skills/autospec-shared/tests/unit/grooming-govern.bats`, `skills/autospec-shared/tests/unit/grooming-observe.bats`. New (3): `tests/autospec/groom-fill.bats`, `tests/autospec/groom-reconcile.bats`, `tests/autospec/test_loop_grooming.bats`. Add any missing and update the "seven grooming bats suites" wording to the new count (ten).

- [ ] **Step 2: Verify ordering** — confirm `check_grooming_contract` is invoked BEFORE `check_ship_completeness` in validate's main sequence (fail() exits, so a later check never runs).

- [ ] **Step 3: Run the grooming contract** — `bash scripts/validate.sh 2>&1 | tee /tmp/val.log`; confirm every grooming suite prints a "running:" line and passes. Parse validate's OWN final status line (a backgrounded run's wrapper exit masks the real result).

- [ ] **Step 4: Update docs** — if v1 shipped a grooming reference doc, add the canary lifecycle: seed→`groom:proposed`→(human adds `auto-implement` | `groom:rejected`)→reconcile→graduation. Note `canary_floor` and the retract-to-canary failure mode.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "test(grooming): register groom-fill/reconcile/loop suites; canary lifecycle docs"`

---

## Self-Review

**Spec coverage:** groom-fill (T2), reconcile (T3), observe rework (T4), promoter canary/auto (T5), loop wiring + reconcile + telemetry + canary_floor (T6), config key + yaml (T1), validate registration + docs (T7). Bootstrap canary = promoter seed path (T5) + graduation via unchanged govern driven by canary_floor (T6). Selection-bias mitigation = conservative canary_floor default (T1) + govern retract-never-below-seed (unchanged). All spec sections covered.

**Placeholder scan:** every code step carries real code; test steps carry real assertions. No TBD/TODO.

**Type consistency:** `groom-fill.sh` output `{ok,body}`/`{ok,reason}` consumed identically in T5; `record_routed` action values `groom-canary`/`groom-auto` produced in T5, consumed in T6; telemetry field set (`template_groomed`,`canary`,`closing_pr`,`outcome`) written in T6, read by reconcile (T3) and observe (T4) consistently; `canary_floor` key name identical in T1/T6.

**Known risk to flag at execution:** the exact bats harness helpers (`setup_promoter_stubs`, the loop test seams) depend on v1's existing suites — the implementer must read the v1 suite in the same file before extending, and reuse its stub conventions rather than inventing new ones. Called out in T5/T6 step 1.
