# Autospec Backlog Auto-Grooming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make raw/discovery-filed GitHub issues automatically `auto-implement`-ready by extending the existing Tier-1.5 promoter with a template-grooming step, a deterministic eligibility gate, safety-gate integration, epic routing, unlabeled intake, and a self-governed `grooming:` config block.

**Architecture:** Extend `scripts/autonomous-promote-open-issues.sh` (already Tier-1.5's `promote-open-issues` command) rather than build a new tier. Deterministic-first (select → safety → classify → eligibility), LLM only for template grooming of issues that need structure. Self-governed via `.autospec/autospec.yml` `grooming:` (replaces the `AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY` env lever), mirroring the advisor ratchet.

**Tech Stack:** POSIX-ish bash (macOS bash 3.2 target, `set -eu`), `gh` CLI, `jq`, python3+PyYAML for config, bats for tests. Reuses `classify-model-fit.sh`, `lint-issue.sh`, `lint-issue-safety.sh`/`issue-safety-gate.sh`, `autospec-split`, and the `advisor-config.sh`/`advisor-govern.sh` patterns.

## Global Constraints

- **bash 3.2 + `set -eu`:** one-sided conditionals use `if/then/fi`, never `[ x ] && cmd`; `grep -c` prints 0 AND exits 1 on empty (guard `|| true` + default); no `RETURN` traps; `[ -f <(...) ]` is false on 3.2 — write process-substitution output to a real temp first; capture `rc` on its own line (`out=$(cmd)`; `rc=$?`) never inline under `&&`.
- **Config precedence:** env > `.autospec/autospec.yml` > built-in default. Env vars (`AUTOSPEC_GROOMING_*`) are CI/test overrides ONLY. Normalize YAML 1.1 booleans (`on`/`off`/`yes`/`no`/`True`/`False`) to `on`/`off` strings (PyYAML parses bare `off` as boolean `False` — the advisor kill-switch bug; mirror `advisor-config.sh`'s True/False→on/off normalization).
- **Fail-closed:** any uncertainty in safety or eligibility → hold/quarantine, never promote. Any error in the pipeline degrades to "promote nothing this cycle" (never a mass-promote).
- **Tests:** TDD, bats. NO live LLM and NO live GitHub mutations in tests — stub `gh` via a PATH shim and stub the groom subagent; assert on emitted JSON and the exact `gh issue edit` argument strings. No DB/external-service mocks (none here).
- **Reuse, don't fork:** call `classify-model-fit.sh`, `lint-issue.sh`, `issue-safety-gate.sh`, `autospec-split` — do not reimplement their logic.
- **Audit:** every promote/hold/quarantine/split decision posts a GitHub comment with the decision + reason (behind apply mode; report-only mode mutates nothing).

---

### Task 1: `grooming-config.sh` — resolve policy + budget from autospec.yml

**Files:**
- Create: `skills/autospec-shared/scripts/grooming-config.sh`
- Test: `skills/autospec-shared/tests/grooming-config.bats`

**Interfaces:**
- Consumes: `.autospec/autospec.yml` (`grooming:` block), env overrides.
- Produces: `grooming-config.sh --key policy` → `auto|on|off`; `--key budget.max_issues_per_cycle` → int; `--key budget.groom_attempts_per_issue` → int. Precedence env>yaml>default. Defaults: policy `auto`, max_issues 5, groom_attempts 2.

- [ ] **Step 1: Write the failing test**

```bash
# skills/autospec-shared/tests/grooming-config.bats
setup() { TMP="$(mktemp -d)"; export AUTOSPEC_CONFIG_FILE="$TMP/autospec.yml"; SCRIPT="${BATS_TEST_DIRNAME}/../scripts/grooming-config.sh"; }
teardown() { rm -rf "$TMP"; }

@test "policy defaults to auto when no config" { printf 'version: 1\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$status" -eq 0 ]; [ "$output" = "auto" ]; }
@test "policy reads yaml value" { printf 'grooming:\n  policy: on\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$output" = "on" ]; }
@test "unquoted off is normalized (not PyYAML boolean False)" { printf 'grooming:\n  policy: off\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key policy; [ "$output" = "off" ]; }
@test "env overrides yaml" { printf 'grooming:\n  policy: off\n' > "$AUTOSPEC_CONFIG_FILE"; AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --key policy; [ "$output" = "auto" ]; }
@test "budget.max_issues_per_cycle default 5" { printf 'version: 1\n' > "$AUTOSPEC_CONFIG_FILE"; run bash "$SCRIPT" --key budget.max_issues_per_cycle; [ "$output" = "5" ]; }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/grooming-config.bats`
Expected: FAIL (script does not exist).

- [ ] **Step 3: Write minimal implementation**

Mirror `skills/autospec-shared/scripts/advisor-config.sh` exactly: resolve `${AUTOSPEC_CONFIG_FILE:-.autospec/autospec.yml}`, read the `grooming:` block via python3+yaml, apply the same True/False→on/off normalization, env override map (`AUTOSPEC_GROOMING_POLICY`, `AUTOSPEC_GROOMING_MAX_ISSUES`, `AUTOSPEC_GROOMING_GROOM_ATTEMPTS`), and the default fallbacks (auto/5/2). Copy the python boolean-normalization block:
```python
if node is True:  print("on"); sys.exit(0)
if node is False: print("off"); sys.exit(0)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/grooming-config.bats`
Expected: PASS (5/5).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/grooming-config.sh skills/autospec-shared/tests/grooming-config.bats
git commit -m "feat(grooming): grooming-config.sh — self-governed policy/budget resolver"
```

---

### Task 2: `list-groomable.sh` — deterministic candidate selection + dedup

**Files:**
- Create: `scripts/list-groomable.sh`
- Test: `tests/autospec/list-groomable.bats`

**Interfaces:**
- Consumes: `gh issue list` (stubbed in tests), `--repo`, `--budget N`.
- Produces: JSON `{"candidates":[{number,title,class}], "skipped":[{number,reason}]}` where `class` ∈ `needs-classify|needs-template|unlabeled`. Oldest-first, capped at budget. Excludes `auto-implement`, `hold:*`, `epic`, `paused-by-user`, `locked-*`, `autospec:needs-human`, `wontfix`, `duplicate`, `security:quarantined`. Dedups against closed no-op findings by a title+body hash (skip reason `dup-of-closed-noop`).

- [ ] **Step 1: Write the failing test** (stub `gh` via PATH shim emitting a fixture issue list)

```bash
# tests/autospec/list-groomable.bats
setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
# stub: `gh issue list ...` → fixture; `gh` anything else → empty
case "$*" in
  *"issue list"*"--state open"*) cat "$GH_ISSUES_FIXTURE" ;;
  *"issue list"*"--state closed"*) printf '%s' "${GH_CLOSED_FIXTURE:-[]}" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/list-groomable.sh"
}
teardown() { rm -rf "$TMP"; }

@test "selects needs-classify, needs-template, and unlabeled; excludes auto-implement/hold" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":10,"title":"fix: x","body":"aaaaaaaaaaaaaaaaaaaa","labels":[{"name":"needs-classify"}]},
 {"number":11,"title":"feat: y","body":"bbbbbbbbbbbbbbbbbbbb","labels":[{"name":"needs-autospec-template"}]},
 {"number":12,"title":"raw","body":"cccccccccccccccccccc","labels":[]},
 {"number":13,"title":"done","body":"dddddddddddddddddddd","labels":[{"name":"auto-implement"}]},
 {"number":14,"title":"held","body":"eeeeeeeeeeeeeeeeeeee","labels":[{"name":"hold:needs-human"}]}]
JSON
  run bash "$SCRIPT" --repo o/r --budget 10
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.candidates[].number] == [10,11,12]'
  echo "$output" | jq -e '.candidates[] | select(.number==11).class == "needs-template"'
}

@test "budget caps candidate count, oldest-first" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":30,"title":"a","body":"aaaaaaaaaaaaaaaaaaaa","labels":[{"name":"needs-classify"}]},
 {"number":20,"title":"b","body":"bbbbbbbbbbbbbbbbbbbb","labels":[{"name":"needs-classify"}]},
 {"number":25,"title":"c","body":"cccccccccccccccccccc","labels":[{"name":"needs-classify"}]}]
JSON
  run bash "$SCRIPT" --repo o/r --budget 2
  echo "$output" | jq -e '[.candidates[].number] == [20,25]'
}
```

- [ ] **Step 2: Run test to verify it fails** — Run: `bats tests/autospec/list-groomable.bats` — Expected: FAIL (no script).

- [ ] **Step 3: Write minimal implementation**

`set -eu`; parse `--repo`/`--budget`; `gh issue list --repo "$repo" --state open --json number,title,labels,body --limit 200`; in a `jq` program: exclude the disqualifying-label set, map remaining to `{number,title,class}` (class: has `needs-autospec-template`→`needs-template`; has `needs-classify`→`needs-classify`; else `unlabeled`), sort by number asc, take budget. For dedup, fetch closed issues once, build a set of `sha256(title+first-200-body-chars)`, drop matches (record skip). Emit the JSON. Guard empty `gh` output → `{"candidates":[],"skipped":[]}` (fail-closed).

- [ ] **Step 4: Run test to verify it passes** — Run: `bats tests/autospec/list-groomable.bats` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/list-groomable.sh tests/autospec/list-groomable.bats
git commit -m "feat(grooming): list-groomable.sh — candidate selection + closed-no-op dedup"
```

---

### Task 3: `promote-eligibility.sh` — deterministic fail-closed eligibility scorer

**Files:**
- Create: `scripts/promote-eligibility.sh`
- Test: `tests/autospec/promote-eligibility.bats`

**Interfaces:**
- Consumes: a body file, `--labels "<csv>"`.
- Produces: JSON `{"decision":"eligible|needs-template|epic|hold","reason":"..."}`. `eligible` = actionable body (≥ MIN_BODY chars) AND a clear `fix:`/`feat:` intent-or-repro AND a bounded scope estimate (heuristic: ≤ N distinct file paths referenced, no "epic"/"multi-subsystem" markers). `epic` = has `epic` label or an epic marker. `needs-template` = groomable but not eligible (multi-file/complex). `hold` = ambiguous / unresolvable-dependency (`Depends on #<nonexistent>`) / too-thin. Fail-closed: unknown → `hold`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/autospec/promote-eligibility.bats
setup() { TMP="$(mktemp -d)"; SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/promote-eligibility.sh"; }
teardown() { rm -rf "$TMP"; }
mkbody() { printf '%s' "$1" > "$TMP/b"; }

@test "clear single-file bug fix is eligible" {
  mkbody "fix: guard set -eu abort in loop.sh line 1250; repro: run conductor with empty backlog, observe crash. Expected: no crash."
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "eligible"'
}
@test "epic label routes to epic" {
  mkbody "big umbrella of work across many subsystems"
  run bash "$SCRIPT" "$TMP/b" --labels "epic,enhancement"
  echo "$output" | jq -e '.decision == "epic"'
}
@test "thin/ambiguous body holds (fail-closed)" {
  mkbody "make it better"
  run bash "$SCRIPT" "$TMP/b" --labels ""
  echo "$output" | jq -e '.decision == "hold"'
}
@test "unresolvable dependency holds" {
  mkbody "fix: something concrete and actionable here with detail. Depends on #999999"
  GH_NONEXISTENT=1 run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "hold" and (.reason|test("depend";"i"))'
}
```

- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL (no script).

- [ ] **Step 3: Write minimal implementation** — `set -eu`; deterministic scoring per the interface. Count file-path-looking tokens for scope; detect `epic` label/marker; detect `Depends on #N` and (in apply contexts) verify N exists via `gh` (test uses `GH_NONEXISTENT`); default branch → `hold`. Emit decision JSON.

- [ ] **Step 4: Run test to verify it passes** — Run: `bats tests/autospec/promote-eligibility.bats` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/promote-eligibility.sh tests/autospec/promote-eligibility.bats
git commit -m "feat(grooming): promote-eligibility.sh — fail-closed legacy-drainable scorer"
```

---

### Task 4: `groom-validate.sh` — template-groom validator + adaptive-retry harness

**Files:**
- Create: `scripts/groom-validate.sh`
- Test: `tests/autospec/groom-validate.bats`

**Interfaces:**
- Consumes: a candidate body file (the LLM-filled body), reuses `scripts/lint-issue.sh` as the template/quality validator.
- Produces: exit 0 + `{"ok":true}` if the body passes `lint-issue.sh`'s template contract; exit 1 + `{"ok":false,"findings":[...]}` carrying the linter findings verbatim (so the caller can feed them back as directives for the next LLM attempt). This script owns ONLY validation — the LLM fill happens in the loop's grooming prose contract (Task 6), because a bash script cannot spawn a subagent (same constraint as the advisor pattern).

- [ ] **Step 1: Write the failing test** (stub `lint-issue.sh` via an injectable path)

```bash
# tests/autospec/groom-validate.bats
setup() {
  TMP="$(mktemp -d)"; SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/groom-validate.sh"
  export AUTOSPEC_LINT_ISSUE_BIN="$TMP/lint.sh"
}
teardown() { rm -rf "$TMP"; }

@test "passes when linter is clean" {
  printf '#!/usr/bin/env bash\nexit 0\n' > "$TMP/lint.sh"; chmod +x "$TMP/lint.sh"
  printf 'well-formed body' > "$TMP/body"
  run bash "$SCRIPT" "$TMP/body"; [ "$status" -eq 0 ]; echo "$output" | jq -e '.ok == true'
}
@test "fails and surfaces findings when linter rejects" {
  printf '#!/usr/bin/env bash\necho "MISSING: ### Primary smoke test" >&2\nexit 1\n' > "$TMP/lint.sh"; chmod +x "$TMP/lint.sh"
  printf 'bad body' > "$TMP/body"
  run bash "$SCRIPT" "$TMP/body"; [ "$status" -eq 1 ]
  echo "$output" | jq -e '.ok == false and (.findings|length > 0)'
}
```

- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL (no script).

- [ ] **Step 3: Write minimal implementation** — `set -eu`; resolve linter as `${AUTOSPEC_LINT_ISSUE_BIN:-$SCRIPT_DIR/lint-issue.sh}`; run it on the body file, capture stdout+stderr and rc (on its own line); emit `{"ok":true}` on rc 0 else `{"ok":false,"findings":[<captured lines>]}`.

- [ ] **Step 4: Run test to verify it passes** — Run: `bats tests/autospec/groom-validate.bats` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/groom-validate.sh tests/autospec/groom-validate.bats
git commit -m "feat(grooming): groom-validate.sh — reuse lint-issue.sh as template validator w/ findings passthrough"
```

---

### Task 5: `grooming-observe.sh` + `grooming-govern.sh` — self-governance ratchet

**Files:**
- Create: `skills/autospec-shared/scripts/grooming-observe.sh`, `skills/autospec-shared/scripts/grooming-govern.sh`
- Test: `skills/autospec-shared/tests/grooming-govern.bats`

**Interfaces:**
- `grooming-observe.sh --telemetry <jsonl>` → `{"groomed_clean_merge_rate":F,"baseline_clean_merge_rate":F,"samples":N}` (a groomed issue is "clean" if its PR merged with no revert/reopen/`escalate:human`).
- `grooming-govern.sh show` → `{"active":["eligible-promote"]}` (seed) …up to `{"active":["eligible-promote","template-promote"]}`. `grooming-govern.sh tick --observed <json> --min-samples N` promotes `eligible-promote → template-promote` when `groomed_rate >= baseline_rate` over the sample floor; retracts on regression; never below seed `eligible-promote`.

- [ ] **Step 1: Write the failing test** (mirror `advisor-govern.bats`)

```bash
# skills/autospec-shared/tests/grooming-govern.bats
setup() { TMP="$(mktemp -d)"; export AUTOSPEC_GROOMING_STATE_DIR="$TMP/state"; G="${BATS_TEST_DIRNAME}/../scripts/grooming-govern.sh"; }
teardown() { rm -rf "$TMP"; }

@test "seed active set is eligible-promote only" { run bash "$G" show; echo "$output" | jq -e '.active == ["eligible-promote"]'; }
@test "promotes template-promote when groomed>=baseline over floor" {
  bash "$G" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":25}' --min-samples 20
  run bash "$G" show; echo "$output" | jq -e '.active == ["eligible-promote","template-promote"]'
}
@test "holds below sample floor" {
  bash "$G" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":5}' --min-samples 20
  run bash "$G" show; echo "$output" | jq -e '.active == ["eligible-promote"]'
}
@test "retracts on regression, never below seed" {
  bash "$G" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":25}' --min-samples 20
  bash "$G" tick --observed '{"groomed_clean_merge_rate":0.5,"baseline_clean_merge_rate":0.8,"samples":25}' --min-samples 20
  run bash "$G" show; echo "$output" | jq -e '.active == ["eligible-promote"]'
}
```

- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Write minimal implementation** — Clone `advisor-govern.sh`'s structure: `ORDER="eligible-promote template-promote"`, `SEED="eligible-promote"`, atomic `write_active` (mktemp+mv), sanitized `read_active`, numeric-guarded `tick` promote/retract via `jq -n '($g >= $b)'`, sample-floor guard (`samples` numeric, `>= min`). `grooming-observe.sh` parses telemetry JSONL with `jq -R 'fromjson? // empty' | jq -s`, computes the two rates + sample count, fail-safe zeros on empty.

- [ ] **Step 4: Run test to verify it passes** — Run: `bats skills/autospec-shared/tests/grooming-govern.bats` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/grooming-observe.sh skills/autospec-shared/scripts/grooming-govern.sh skills/autospec-shared/tests/grooming-govern.bats
git commit -m "feat(grooming): self-governance ratchet (observe + govern) mirroring advisor"
```

---

### Task 6: Extend `autonomous-promote-open-issues.sh` to orchestrate the grooming pipeline

**Files:**
- Modify: `scripts/autonomous-promote-open-issues.sh`
- Test: `tests/autospec/autonomous-promote-open-issues.bats` (extend existing suite if present; else create)

**Interfaces:**
- Consumes: `grooming-config.sh` (policy/budget), `list-groomable.sh`, `issue-safety-gate.sh` (safety), `classify-model-fit.sh`, `promote-eligibility.sh`, `groom-validate.sh`, `grooming-govern.sh show` (active set), `/autospec-split` (epic, via its helper if scriptable else emit a `route:split` action for the loop).
- Produces: same JSON envelope as today (`{dry,filed,promoted,skipped,reason}`) plus a `held`/`quarantined`/`routed` breakdown. Replaces the `AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY` double-gate with: apply iff `grooming-config policy` ∈ `auto|on` (env override still honored for CI). Per candidate: safety → (quarantine|continue); classify; eligibility → `eligible`(promote now) | `needs-template`(if `template-promote` in govern active set → emit groom contract for the loop, else hold) | `epic`(route to split) | `hold`. Every mutation posts an audit comment.

- [ ] **Step 1: Write the failing test** (stub all sub-scripts + `gh` via PATH shims; assert routing decisions on fixtures)

```bash
@test "eligible needs-classify issue is promoted with audit comment" {
  # stubs: safety→PASS, classify→{ctx,reasoning}, eligibility→eligible; gh records edits
  run bash "$SCRIPT" --repo o/r --apply
  grep -q 'add-label auto-implement' "$GH_LOG"
  grep -q 'remove-label needs-classify' "$GH_LOG"
  grep -q 'issue comment' "$GH_LOG"   # audit
}
@test "SAFETY_BLOCK quarantines and never promotes" {
  # stub safety→SAFETY_BLOCK
  run bash "$SCRIPT" --repo o/r --apply
  grep -q 'add-label security:quarantined' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
}
@test "needs-template held when template-promote not in active govern set" {
  # stub eligibility→needs-template, govern show→["eligible-promote"]
  run bash "$SCRIPT" --repo o/r --apply
  grep -q 'add-label hold:needs-human' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
}
@test "policy off mutates nothing" {
  AUTOSPEC_GROOMING_POLICY=off run bash "$SCRIPT" --repo o/r --apply
  [ ! -s "$GH_LOG" ] || ! grep -q 'issue edit' "$GH_LOG"
  echo "$output" | jq -e '.dry == true'
}
```

- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Write implementation** — Refactor the candidate loop: resolve policy via `grooming-config.sh` (drop the `AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY` requirement; keep an env override seam for tests); source candidates from `list-groomable.sh` (adds unlabeled + needs-template classes) instead of the raw `needs-classify`-only query; per candidate run safety → classify → eligibility → route; gate `needs-template` promotion on `grooming-govern.sh show` active set; post audit comments; keep the report-only path (`policy off`/no-apply → dry JSON, no mutation). Preserve back-compat of the output envelope.

- [ ] **Step 4: Run test to verify it passes** — Run: `bats tests/autospec/autonomous-promote-open-issues.bats` — Expected: PASS. Then `autospec validate`.

- [ ] **Step 5: Commit**

```bash
git add scripts/autonomous-promote-open-issues.sh tests/autospec/autonomous-promote-open-issues.bats
git commit -m "feat(grooming): orchestrate safety→classify→eligibility→promote/groom/split/hold; policy-gated"
```

---

### Task 7: Wire the grooming pipeline + telemetry into the Tier-1.5 loop call site

**Files:**
- Modify: `scripts/lib/autospec-loop.sh` (around line 1073, the `_promote_cmd` construction)
- Test: `tests/autospec/test_conductor_wiring.bats` (extend)

**Interfaces:**
- Consumes: waterfall `tier=1.5` emit (unchanged).
- Produces: the loop invokes the extended promoter, appends one telemetry record per grooming cycle to the grooming telemetry JSONL (`{ts,promoted,held,quarantined,routed}`), and after the cycle calls `grooming-govern.sh tick` with `grooming-observe.sh` output (only when policy=auto). The `groom-to-template` LLM fill for `needs-template` candidates is the loop's prose contract: for each candidate the loop marks for grooming, dispatch a Tier-B subagent to fill the template sections, then re-run `groom-validate.sh` (≤ `groom_attempts_per_issue` retries feeding findings back), then hand back to the promoter to finalize. (Same script-owns-bookkeeping / model-call-is-prose split as the advisor pattern.)

- [ ] **Step 1: Write the failing test** — extend the conductor-wiring bats: with a `tier=1.5` waterfall emit and stubbed promoter, assert the loop calls the grooming promoter and appends a telemetry line; with `policy=auto` assert a govern `tick` is invoked.

- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Write implementation** — At the Tier-1.5 branch, keep the existing `_promote_cmd` path but (a) pass policy through, (b) after the promoter returns, append the telemetry record and (policy=auto) run the observe→govern tick. Add the prose grooming-contract block (documented steps the loop follows for `needs-template` candidates). Honor bash 3.2/`set -eu` (the conductor already runs under it — reuse the `if/then/else` rc-capture pattern #1625 established).

- [ ] **Step 4: Run test to verify it passes** — Run: `bats tests/autospec/test_conductor_wiring.bats` then `autospec validate`.

- [ ] **Step 5: Commit**

```bash
git add scripts/lib/autospec-loop.sh tests/autospec/test_conductor_wiring.bats
git commit -m "feat(grooming): wire grooming pipeline + govern tick into Tier-1.5 loop"
```

---

### Task 8: `grooming:` config schema/validator + autospec.yml example + docs (+ trio if touched)

**Files:**
- Modify: the autospec.yml schema/validator (locate via `grep -rl 'improvement_budget\|continuous_improvement' scripts/`), `.autospec/autospec.yml`, `examples/` if a config example exists.
- Modify (only if a SKILL.md prose section documents Tier 1.5): `skills/autospec-run/SKILL.md` → then MANDATORY `scripts/derive-trio.sh skills/autospec-run --in-place` + `scripts/gen-skill-goldens.sh autospec-run` in the SAME commit.
- Test: extend the config-schema bats/validator test.

**Interfaces:**
- Consumes: `.autospec/autospec.yml`.
- Produces: `grooming: {policy, budget:{max_issues_per_cycle, groom_attempts_per_issue}}` is a recognized, validated section (unknown keys rejected; policy ∈ auto|on|off; budgets positive ints).

- [ ] **Step 1: Write the failing test** — schema test: a valid `grooming:` block passes; `policy: bogus` fails; negative budget fails.
- [ ] **Step 2: Run test to verify it fails** — Expected: FAIL.
- [ ] **Step 3: Write implementation** — add the `grooming:` section to the schema/validator (mirror `continuous_improvement`/`improvement_budget`); add a commented `grooming:` block to `.autospec/autospec.yml` with the defaults; if SKILL.md prose is edited to document the tier, run derive-trio + gen-skill-goldens and stage all trio + golden outputs (a prose-only trio commit fails `validate.sh` closed).
- [ ] **Step 4: Run test to verify it passes** — Run the schema test, then `autospec validate` (must be fully green, including `check_derive_trio_consistency` if the trio was touched).
- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(grooming): autospec.yml grooming schema + example + docs"
```

---

## Self-Review

**Spec coverage:** select (T2) ✓, safety floor (T6 via issue-safety-gate) ✓, classify (T6 reuse) ✓, eligibility B-gate (T3) ✓, LLM groom-to-template A-fallback (T4 validator + T7 prose contract) ✓, epic→split (T6) ✓, unlabeled intake (T2 class + T6) ✓, self-governance (T5) ✓, config migration off env lever (T1 + T8) ✓, audit comments (T6) ✓, guardrails/fail-closed (T2/T3/T6) ✓, wiring + telemetry (T7) ✓. All spec sections map to a task.

**Placeholder scan:** no TBD/TODO; every code step carries real test or implementation content or a concrete grep to locate the edit point.

**Type/interface consistency:** `grooming-govern.sh` active-set names (`eligible-promote`/`template-promote`) are used identically in T5 and T6; `list-groomable.sh` `class` values (`needs-classify|needs-template|unlabeled`) are consumed unchanged in T6; the promoter output envelope (`{dry,filed,promoted,skipped,...}`) is preserved from the existing script. The one deliberately-deferred detail (T8's exact schema-validator file) carries a `grep` to locate it at implementation time, not a guessed path.

## Execution Handoff

Plan saved to `docs/plans/2026-07-09-autospec-backlog-grooming.md`. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, task review between tasks, broad final review. Fits this plan (8 mostly-independent deterministic scripts + two integration tasks).
2. **Inline Execution** — execute tasks in this session with checkpoints.
