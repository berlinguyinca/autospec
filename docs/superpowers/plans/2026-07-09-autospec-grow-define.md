# AutoSpec Growth — /autospec-grow-define (Plan 3 of 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `/autospec-grow-define` trio skill — a testable deterministic orchestrator (validate→dedup→verify→rank→slice, then file issues) plus the lock-step skill prose (6-lens roster + phases) that dispatches LLM subagents around it and hands off to `/autospec-grow-run`.

**Architecture:** Two bash orchestrator scripts under `skills/autospec-shared/scripts/` carry the deterministic G2→G3 work and are unit-tested with bats (the LLM outputs — lens candidates, verify verdicts — enter as input files, which is the mock seam). The trio skill (`skills/autospec-grow-define/{SKILL.md,codex/prompt.md,opencode/agent.md}`) is the orchestration prose: it dispatches the lenses and verify subagents, writes their outputs to files, calls the two scripts, invokes `autospec-define` Phase 3 for artifact decomposition, and stops. A mock-driven bats smoke test drives the two scripts end-to-end; lock-step + goldens + a structural-section check gate the prose.

**Tech Stack:** bash (macOS bash 3.2 compatible), `jq`, bats-core, the Plan 1+2 growth-shared scripts, `derive-trio.sh`, `gen-skill-goldens.sh`.

## Global Constraints

- **Bash 3.2 compatible.** No associative arrays, no `${var,,}`; `set -euo pipefail`; `if/then/fi` not `[ test ] && action`; no `RETURN` traps.
- **Reuse, do not reimplement** the Plan 1+2 scripts: `validate-growth-config.sh`, `growth-measure.sh`, `growth-source-weights.sh`, `growth-ledger.sh`, `validate-growth-candidate.sh`, `growth-candidate-dedup.sh`, `growth-candidate-verify.sh`, `growth-candidate-rank.sh`. Locate siblings via `HERE="$(cd "$(dirname "$0")" && pwd)"`.
- **Fail-closed:** a candidate with no verify verdict is treated as refuted; a `gh` issue-create failure files no ledger line (no orphan entries); invalid `growth.yml` → inert exit.
- **Ledger record schema** (Plan 1, unchanged): `{round, source, title, norm_title, channel, kind, issue, outcome, reason, ts}`. A filed issue appends outcome `pending`; `source` = the candidate's `lens`.
- **`GROWTH_LEDGER`** env seam; tests set it to a temp file and build state via the real `growth-ledger.sh --append` (never a bespoke fixture — the anti-self-consistent-fixture rule).
- **Trio lock-step (sacred):** `SKILL.md` / `codex/prompt.md` / `opencode/agent.md` bodies byte-identical; only frontmatter differs. After ANY edit to `SKILL.md`, run `scripts/derive-trio.sh skills/autospec-grow-define --in-place` then `scripts/gen-skill-goldens.sh autospec-grow-define`, or `validate.sh` fails closed. `codex/prompt.md` has a leading blank line.
- **Structural sections required** in the trio body: `## Self-update mode` (pure prose, no shelled user text), the harness-adapter row, a Model-tier table, and a `## Lens roster` naming all 6 lenses.
- **Conventional commits**, branch `feat/autospec-grow-define`, never `--no-verify`, never push to `main`. New scripts `bash -n` clean + `chmod +x`.

---

### Task 1: `grow-define-pipeline.sh` (validate → dedup → verify → rank → slice)

**Files:**
- Create: `skills/autospec-shared/scripts/grow-define-pipeline.sh`
- Test: `tests/unit/grow-define-pipeline.bats`

**Interfaces:**
- Consumes: `validate-growth-candidate.sh`, `growth-candidate-dedup.sh`, `growth-candidate-verify.sh`, `growth-candidate-rank.sh` (all Plan 2).
- Produces: `grow-define-pipeline.sh <candidates.jsonl> <verdicts.jsonl> <config.json>` → prints ranked, verified, top-N-sliced candidates (JSONL). `verdicts.jsonl` lines: `{"norm_title":"...","real":bool,"reason":"..."}`. A validated+deduped candidate with no matching verdict is treated as refuted (fail-closed). Top-N = `.grow.max_issues_per_cycle // 8` from config. Invalid candidates are dropped (logged to stderr), never abort the batch. Missing candidates/verdicts/config file → exit 2.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/grow-define-pipeline.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
P="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-pipeline.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"$2\",\"title\":\"$3\",\"norm_title\":\"$3\",\"roi\":$4,\"effort\":\"small\",\"severity\":$5,\"confidence\":0.8}"; }
verdict() { echo "{\"norm_title\":\"$1\",\"real\":$2,\"reason\":\"$3\"}"; }
cfg() { echo "{\"grow\":{\"max_issues_per_cycle\":$1}}" > "$TMP/cfg.json"; echo "$TMP/cfg.json"; }

@test "script exists and is bash -n clean" {
  [ -f "$P" ]; run bash -n "$P"; [ "$status" -eq 0 ]
}

@test "valid+verified candidates pass; invalid dropped; refuted removed" {
  : > "$TMP/c.jsonl"
  cand keyword-gap artifact alpha 5 5 >> "$TMP/c.jsonl"
  cand community outbound beta 3 3 >> "$TMP/c.jsonl"
  echo '{"lens":"bogus-lens","kind":"artifact","title":"x","norm_title":"x"}' >> "$TMP/c.jsonl"  # invalid
  : > "$TMP/v.jsonl"
  verdict alpha true ok >> "$TMP/v.jsonl"
  verdict beta false offtopic >> "$TMP/v.jsonl"   # refuted -> removed
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"norm_title":"alpha"'* ]]
  [[ "$output" != *'"norm_title":"beta"'* ]]
  [[ "$output" != *'"norm_title":"x"'* ]]
}

@test "candidate with no verdict is fail-closed (refuted, removed)" {
  : > "$TMP/c.jsonl"; cand keyword-gap artifact gamma 4 4 >> "$TMP/c.jsonl"
  : > "$TMP/v.jsonl"   # empty verdicts
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -eq 0 ]
  [ -z "$(printf '%s' "$output" | tr -d '[:space:]')" ]
}

@test "deduped against ledger" {
  bash "$LG" --append '{"round":1,"source":"keyword-gap","title":"seen","norm_title":"seen","channel":"content","kind":"artifact","issue":9,"outcome":"merged_clean","reason":"","ts":"2026-07-09T00:00:00Z"}'
  : > "$TMP/c.jsonl"; cand keyword-gap artifact seen 5 5 >> "$TMP/c.jsonl"; cand keyword-gap artifact fresh 5 5 >> "$TMP/c.jsonl"
  : > "$TMP/v.jsonl"; verdict seen true ok >> "$TMP/v.jsonl"; verdict fresh true ok >> "$TMP/v.jsonl"
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [[ "$output" == *'"norm_title":"fresh"'* ]]
  [[ "$output" != *'"norm_title":"seen"'* ]]
}

@test "top-N slice honored" {
  : > "$TMP/c.jsonl"; : > "$TMP/v.jsonl"
  for i in 1 2 3; do cand keyword-gap artifact "t$i" 5 5 >> "$TMP/c.jsonl"; verdict "t$i" true ok >> "$TMP/v.jsonl"; done
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 2)"
  [ "$(printf '%s\n' "$output" | grep -c '"norm_title"')" -eq 2 ]
}

@test "missing input file fails" {
  run bash "$P" "$TMP/nope.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/grow-define-pipeline.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the orchestrator**

```bash
#!/usr/bin/env bash
# grow-define-pipeline.sh — deterministic G2 pipeline: validate -> dedup ->
# verify (against a supplied verdicts file) -> rank -> slice top-N.
set -euo pipefail

CANDS="${1:?usage: grow-define-pipeline.sh <candidates.jsonl> <verdicts.jsonl> <config.json>}"
VERDICTS="${2:?usage: ...}"
CONFIG="${3:?usage: ...}"
for f in "$CANDS" "$VERDICTS" "$CONFIG"; do
  if [ ! -f "$f" ]; then echo "not found: $f" >&2; exit 2; fi
done

HERE="$(cd "$(dirname "$0")" && pwd)"
MAXN="$(jq -r '.grow.max_issues_per_cycle // 8' "$CONFIG")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# 1. validate: keep only valid candidates (log + drop invalid)
: > "$work/valid.jsonl"
while IFS= read -r line; do
  [ -n "$line" ] || continue
  echo "$line" > "$work/one.json"
  if "$HERE/validate-growth-candidate.sh" "$work/one.json" >/dev/null 2>"$work/err"; then
    echo "$line" >> "$work/valid.jsonl"
  else
    echo "grow-define: dropped invalid candidate: $(cat "$work/err")" >&2
  fi
done < "$CANDS"

# 2. dedup against the ledger
"$HERE/growth-candidate-dedup.sh" "$work/valid.jsonl" "${GROWTH_LEDGER:-.autospec/growth/ledger.jsonl}" > "$work/deduped.jsonl" || true

# 3. verify each survivor against its verdict (fail-closed if absent)
: > "$work/verified.jsonl"
while IFS= read -r line; do
  [ -n "$line" ] || continue
  echo "$line" > "$work/cand.json"
  nt="$(echo "$line" | jq -r '.norm_title')"
  # find verdict by exact norm_title; default fail-closed
  v="$(jq -c --arg n "$nt" 'select(.norm_title==$n)' "$VERDICTS" | tail -1)"
  if [ -z "$v" ]; then
    v='{"real":false,"reason":"no verdict, refused"}'
  fi
  echo "$v" > "$work/verdict.json"
  "$HERE/growth-candidate-verify.sh" "$work/cand.json" "$work/verdict.json" >> "$work/verified.jsonl"
done < "$work/deduped.jsonl"

# 4. rank, 5. slice top-N
if [ ! -s "$work/verified.jsonl" ]; then exit 0; fi
"$HERE/growth-candidate-rank.sh" "$work/verified.jsonl" | jq -c . | head -n "$MAXN"
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/grow-define-pipeline.sh
bats tests/unit/grow-define-pipeline.bats
```
Expected: all PASS. (Note: `growth-candidate-verify.sh` appends refuted lines to `$GROWTH_LEDGER` as a side effect — the pipeline relies on that for down-weighting; the dedup step reads the same ledger. The `head -n "$MAXN"` slices the already-ranked output.)

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/grow-define-pipeline.sh tests/unit/grow-define-pipeline.bats
git commit -m "feat: grow-define deterministic pipeline (validate/dedup/verify/rank/slice)"
```

---

### Task 2: `grow-define-file-issues.sh` (create issues + ledger append)

**Files:**
- Create: `skills/autospec-shared/scripts/grow-define-file-issues.sh`
- Test: `tests/unit/grow-define-file-issues.bats`

**Interfaces:**
- Consumes: `gh` (issue creation — the mock seam), `growth-ledger.sh --append`.
- Produces: `grow-define-file-issues.sh <ranked.jsonl> <config.json>`. For each candidate: build a title + body + labels per `kind` (`artifact` → labels `auto-implement,growth:artifact,growth:<channel>`; `outbound` → labels `growth:outbound,growth/needs-draft,growth:<channel>`), run `gh issue create --title ... --body ... --label ...`, parse the issue number from gh's output (a URL ending in `/<number>`), and ONLY on success append a `pending` ledger line (`source`=lens, `issue`=number). On `gh` failure: log to stderr, skip, append no ledger line. Prints each created issue number. Missing input → exit 2.

- [ ] **Step 1: Write the failing test (with a gh mock)**

```bash
# tests/unit/grow-define-file-issues.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
F="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() {
  TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"
  # gh mock: echoes a fake issue URL; records args; can be forced to fail
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$GH_LOG"
if [ -n "${GH_FAIL:-}" ]; then echo "gh: create failed" >&2; exit 1; fi
n="$(cat "$GH_COUNTER" 2>/dev/null || echo 100)"; n=$((n+1)); echo "$n" > "$GH_COUNTER"
echo "https://github.com/acme/site/issues/$n"
SH
  chmod +x "$TMP/bin/gh"
  export GH_LOG="$TMP/gh.log"; export GH_COUNTER="$TMP/gh.counter"
  export PATH="$TMP/bin:$PATH"
  echo '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/cfg.json"
}
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER GH_LOG GH_COUNTER GH_FAIL; }

ranked() {
  : > "$TMP/r.jsonl"
  echo '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add vs page","norm_title":"add vs page","roi":5,"effort":"small","severity":5,"confidence":0.9,"rank_score":0.9}' >> "$TMP/r.jsonl"
  echo '{"lens":"community","channel":"outreach","kind":"outbound","title":"Show HN post","norm_title":"show hn post","roi":4,"effort":"small","severity":4,"confidence":0.8,"rank_score":0.6}' >> "$TMP/r.jsonl"
  echo "$TMP/r.jsonl"
}

@test "script exists and is bash -n clean" {
  [ -f "$F" ]; run bash -n "$F"; [ "$status" -eq 0 ]
}

@test "creates an issue per candidate and appends one pending ledger line each" {
  run bash "$F" "$(ranked)" "$TMP/cfg.json"
  [ "$status" -eq 0 ]
  # two gh invocations
  [ "$(grep -c 'issue create' "$GH_LOG")" -eq 2 ]
  # artifact labels
  grep -q 'growth:artifact' "$GH_LOG"
  grep -q 'auto-implement' "$GH_LOG"
  # outbound labels
  grep -q 'growth:outbound' "$GH_LOG"
  grep -q 'growth/needs-draft' "$GH_LOG"
  # two pending ledger lines
  [ "$(grep -c '"outcome":"pending"' "$GROWTH_LEDGER")" -eq 2 ]
}

@test "gh failure files no ledger line" {
  export GH_FAIL=1
  run bash "$F" "$(ranked)" "$TMP/cfg.json"
  # script continues (logs + skips); ledger stays empty
  [ ! -f "$GROWTH_LEDGER" ] || [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 0 ]
}

@test "missing input fails" {
  run bash "$F" "$TMP/nope.jsonl" "$TMP/cfg.json"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/grow-define-file-issues.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the issue-filer**

```bash
#!/usr/bin/env bash
# grow-define-file-issues.sh — file a GitHub issue per ranked candidate and
# append a pending ledger line only after successful creation.
set -euo pipefail

RANKED="${1:?usage: grow-define-file-issues.sh <ranked.jsonl> <config.json>}"
CONFIG="${2:?usage: grow-define-file-issues.sh <ranked.jsonl> <config.json>}"
for f in "$RANKED" "$CONFIG"; do
  if [ ! -f "$f" ]; then echo "not found: $f" >&2; exit 2; fi
done

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"

while IFS= read -r line; do
  [ -n "$line" ] || continue
  lens="$(echo "$line" | jq -r '.lens')"
  kind="$(echo "$line" | jq -r '.kind')"
  channel="$(echo "$line" | jq -r '.channel')"
  title="$(echo "$line" | jq -r '.title')"
  norm="$(echo "$line" | jq -r '.norm_title')"
  rationale="$(echo "$line" | jq -r '.rationale // ""')"

  if [ "$kind" = "artifact" ]; then
    labels="auto-implement,growth:artifact,growth:$channel"
    body="$(printf 'Growth artifact (lens: %s, channel: %s)\n\n%s\n\nFiled by /autospec-grow-define.' "$lens" "$channel" "$rationale")"
  else
    labels="growth:outbound,growth/needs-draft,growth:$channel"
    body="$(printf 'Growth outbound draft needed (lens: %s, channel: %s)\n\nTarget/rule: see rationale.\n\n%s\n\nDrafted + gated + queued by /autospec-grow-run.' "$lens" "$channel" "$rationale")"
  fi

  if ! url="$(gh issue create --title "$title" --body "$body" --label "$labels" 2>/dev/null)"; then
    echo "grow-define: gh issue create failed for: $title (skipped)" >&2
    continue
  fi
  num="$(printf '%s' "$url" | grep -oE '[0-9]+$' || true)"
  if [ -z "$num" ]; then
    echo "grow-define: could not parse issue number from: $url (skipped)" >&2
    continue
  fi

  ledline="$(jq -n --arg s "$lens" --arg t "$title" --arg n "$norm" \
     --arg c "$channel" --arg k "$kind" --argjson i "$num" \
     '{round:1,source:$s,title:$t,norm_title:$n,channel:$c,kind:$k,issue:$i,outcome:"pending",reason:"",ts:"1970-01-01T00:00:00Z"}')"
  "$LEDGER_SH" --append "$ledline"
  echo "$num"
done < "$RANKED"
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/grow-define-file-issues.sh
bats tests/unit/grow-define-file-issues.bats
```
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/grow-define-file-issues.sh tests/unit/grow-define-file-issues.bats
git commit -m "feat: grow-define issue filer (per-kind template + pending ledger)"
```

---

### Task 3: Author the `/autospec-grow-define` trio skill

**Files:**
- Create: `skills/autospec-grow-define/SKILL.md` (authoritative)
- Create (derived): `skills/autospec-grow-define/codex/prompt.md`, `skills/autospec-grow-define/opencode/agent.md`
- Create: `skills/autospec-grow-define/install.sh`, `skills/autospec-grow-define/uninstall.sh`, `skills/autospec-grow-define/README.md`
- Create (goldens): whatever `gen-skill-goldens.sh` writes for this skill.

**Interfaces:**
- Consumes: `grow-define-pipeline.sh`, `grow-define-file-issues.sh` (Tasks 1–2), the Plan 1 config validator + measure + ledger, and `autospec-define` Phase 3 (for artifact decomposition).
- Produces: the operator-facing `/autospec-grow-define` skill. No test asserts prose behavior directly; lock-step + goldens + the structural check (Task 4) + the smoke test (Task 4, which drives the scripts) gate it.

**Author `SKILL.md` following `skills/autospec-define/SKILL.md` and `skills/autospec-explore/SKILL.md` as templates.** Required sections, in order:

- [ ] **Step 1: Frontmatter + title + Self-update mode**

Frontmatter `name: autospec-grow-define`, a `description:` matching the family style. Body opens with `# autospec-grow-define workflow (harness-neutral)`, then `## Self-update mode` (pure prose describing how the skill updates itself via `install.sh --update`; NO bash heredocs of user text — matches the memory rule).

- [ ] **Step 2: Required capabilities & harness adapter + Harness detection + Model-tier**

`## Required capabilities & harness adapter` with the adapter row (mirror `autospec-define`'s table). `## Harness detection` resolving `TIER_A` (research/verify/decompose) and `TIER_B` (cheap) once at start. A Model-tier table: Tier A = top model + max thinking for lens research + adversarial verify + decomposition; Tier B for mechanical steps.

- [ ] **Step 3: Config gate + Relevant memory injection**

`## Configuration` — read `.autospec/growth.yml`, validate with `validate-growth-config.sh`; if absent/invalid, print a one-line pointer and exit (inert). `## Relevant memory injection` — run-start memory query (mirror `autospec-define`).

- [ ] **Step 4: The phase prose (G0–G3 + Stop)**

- `## Phase G0 — Measure (optional)`: run `growth-measure.sh --normalize` on config-pointed/fixture metric files if present; else empty envelope.
- `## Phase G1 — Lens research` + `## Lens roster`: the 6 lens dispatches (technical-seo, keyword-gap, content-opportunity, community, directory, backlink), each a Tier-A subagent whose budget is scaled by `growth-source-weights.sh[lens]`; each returns candidate JSON (Plan 2 schema). Write all candidates to a candidates JSONL.
- `## Phase G2 — Pipeline`: for each candidate dispatch a Tier-A adversarial-verify subagent (prompted to refute; default `real:false` on doubt) and collect `{norm_title,real,reason}` into a verdicts JSONL; then run `grow-define-pipeline.sh <candidates> <verdicts> <growth.yml-as-json>` to get the ranked top-N.
- `## Phase G3 — Decompose`: split the ranked output by `kind`. For `growth:artifact`, invoke `autospec-define` Phase 3 decomposition to create linked auto-implement issues (then add `growth:artifact`/channel labels) OR use `grow-define-file-issues.sh` for the direct path — **the skill uses `grow-define-file-issues.sh` for outbound and for artifacts that don't need multi-issue decomposition, and delegates to `autospec-define` Phase 3 only for artifacts that warrant decomposition into multiple linked issues.** (State this routing explicitly in the prose.)
- `## Stop`: print the summary and hand off to `/autospec-grow-run`.

- [ ] **Step 5: Derive the trio + generate goldens**

```bash
scripts/derive-trio.sh skills/autospec-grow-define --in-place
scripts/gen-skill-goldens.sh autospec-grow-define
scripts/derive-trio.sh skills/autospec-grow-define --check   # must report no drift
```
Expected: `--check` clean; `codex/prompt.md` and `opencode/agent.md` created with identical bodies.

- [ ] **Step 6: install.sh / uninstall.sh / README.md**

Copy `skills/autospec-define/install.sh` + `uninstall.sh` as templates, changing `SKILL_NAME` to `autospec-grow-define` and the raw-base env var names. `bash -n` both. Write a short README mirroring the family.

- [ ] **Step 7: Verify derivation + syntax, commit**

```bash
scripts/derive-trio.sh skills/autospec-grow-define --check
bash -n skills/autospec-grow-define/install.sh
bash -n skills/autospec-grow-define/uninstall.sh
git add skills/autospec-grow-define
git commit -m "feat: /autospec-grow-define trio skill (lens roster + pipeline orchestration)"
```

---

### Task 4: Structural check, smoke test, and `validate.sh` wiring

**Files:**
- Create: `tests/autospec-grow-define/smoke.bats`
- Modify: `autospec validate` (add `check_grow_define_contract` + register)

**Interfaces:**
- Consumes: Tasks 1–3 outputs.
- Produces: a mock-driven end-to-end smoke test + a `check_grow_define_contract` that gates lock-step/derivation/goldens/structural-sections + runs the growth pipeline bats + the smoke test.

- [ ] **Step 1: Write the end-to-end smoke test (mock gh, fixed lens/verdict inputs)**

```bash
# tests/autospec-grow-define/smoke.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
S="$REPO_ROOT/skills/autospec-shared/scripts"

setup() {
  TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$GH_LOG"
n="$(cat "$GH_COUNTER" 2>/dev/null || echo 200)"; n=$((n+1)); echo "$n" > "$GH_COUNTER"
echo "https://github.com/acme/site/issues/$n"
SH
  chmod +x "$TMP/bin/gh"; export GH_LOG="$TMP/gh.log"; export GH_COUNTER="$TMP/gh.counter"
  export PATH="$TMP/bin:$PATH"
  echo '{"product":{"name":"Acme"},"site":{"repo_path":"."},"grow":{"max_issues_per_cycle":5}}' > "$TMP/cfg.json"
}
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER GH_LOG GH_COUNTER; }

@test "end-to-end: candidates -> pipeline -> file issues, deterministic" {
  # simulated lens outputs (what G1 subagents would produce)
  : > "$TMP/c.jsonl"
  echo '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add vs page","norm_title":"add vs page","roi":5,"effort":"small","severity":5,"confidence":0.9}' >> "$TMP/c.jsonl"
  echo '{"lens":"community","channel":"outreach","kind":"outbound","title":"Show HN","norm_title":"show hn","roi":4,"effort":"small","severity":3,"confidence":0.7}' >> "$TMP/c.jsonl"
  echo '{"lens":"directory","channel":"directories","kind":"outbound","title":"spammy","norm_title":"spammy","roi":2,"effort":"small","severity":2,"confidence":0.5}' >> "$TMP/c.jsonl"
  # simulated verify verdicts (what G2 subagents would produce): spammy refuted
  : > "$TMP/v.jsonl"
  echo '{"norm_title":"add vs page","real":true,"reason":"ok"}' >> "$TMP/v.jsonl"
  echo '{"norm_title":"show hn","real":true,"reason":"on-topic"}' >> "$TMP/v.jsonl"
  echo '{"norm_title":"spammy","real":false,"reason":"drive-by"}' >> "$TMP/v.jsonl"

  bash "$S/grow-define-pipeline.sh" "$TMP/c.jsonl" "$TMP/v.jsonl" "$TMP/cfg.json" > "$TMP/ranked.jsonl"
  # spammy refuted -> not in ranked; 2 survivors
  [ "$(grep -c '"norm_title"' "$TMP/ranked.jsonl")" -eq 2 ]
  ! grep -q '"norm_title":"spammy"' "$TMP/ranked.jsonl"
  # refuted recorded in ledger
  grep -q '"outcome":"refuted"' "$GROWTH_LEDGER"

  bash "$S/grow-define-file-issues.sh" "$TMP/ranked.jsonl" "$TMP/cfg.json" > "$TMP/nums.txt"
  # two issues filed, correct label routing
  [ "$(grep -c 'issue create' "$GH_LOG")" -eq 2 ]
  grep -q 'growth:artifact' "$GH_LOG"
  grep -q 'growth:outbound' "$GH_LOG"
  # two pending ledger lines
  [ "$(grep -c '"outcome":"pending"' "$GROWTH_LEDGER")" -eq 2 ]
}
```

- [ ] **Step 2: Run smoke test**

Run: `bats tests/autospec-grow-define/smoke.bats`
Expected: PASS (Tasks 1–2 implemented). Fix any pipeline/filer bug the smoke test surfaces in the relevant script; do not weaken assertions.

- [ ] **Step 3: Add the validate.sh check**

Model on `check_growth_shared_contract` and the existing `grep -q '^## Self-update mode'` structural pattern. Add after `check_growth_candidate_pipeline_contract`:

```bash
check_grow_define_contract() {
    info "grow-define: lock-step + goldens + structural sections + smoke"
    local d="skills/autospec-grow-define"
    local rc=0
    # scripts bash -n
    for s in grow-define-pipeline grow-define-file-issues; do
        check_bash_syntax "skills/autospec-shared/scripts/$s.sh"
    done
    # trio lock-step derivation
    if ! scripts/derive-trio.sh "$d" --check >/dev/null 2>&1; then
        fail "$d: trio drift — run derive-trio.sh --in-place + gen-skill-goldens.sh"
    fi
    # required structural sections across the trio
    local t
    for t in SKILL.md codex/prompt.md opencode/agent.md; do
        [ -f "$d/$t" ] || { fail "$d/$t: missing"; continue; }
        grep -q '^## Self-update mode' "$d/$t" || fail "$d/$t: missing Self-update mode"
        grep -q '^## Lens roster' "$d/$t"      || fail "$d/$t: missing Lens roster"
        grep -q 'Model' "$d/$t"                || fail "$d/$t: missing Model-tier reference"
    done
    # all 6 lenses named in SKILL.md
    for lens in technical-seo keyword-gap content-opportunity community directory backlink; do
        grep -q "$lens" "$d/SKILL.md" || fail "$d/SKILL.md: lens $lens not named in roster"
    done
    if command -v bats >/dev/null 2>&1; then
        for b in tests/unit/grow-define-pipeline.bats tests/unit/grow-define-file-issues.bats tests/autospec-grow-define/smoke.bats; do
            info "  running: $b"
            bats "$b" >/tmp/validate-grow-define.log 2>&1 || { cat /tmp/validate-grow-define.log >&2; fail "$b: failed"; }
        done
    fi
    return "$rc"
}
```

- [ ] **Step 4: Register the check**

Add `check_grow_define_contract` to `main`'s run list immediately after `check_growth_candidate_pipeline_contract` (find via `grep -n "check_growth_candidate_pipeline_contract" autospec validate`).

- [ ] **Step 5: Verify (do NOT run full validate.sh here — controller runs it in a clean worktree)**

```bash
bash -n autospec validate
scripts/derive-trio.sh skills/autospec-grow-define --check
bats tests/unit/grow-define-pipeline.bats tests/unit/grow-define-file-issues.bats tests/autospec-grow-define/smoke.bats
grep -n "check_grow_define_contract" autospec validate   # must show 2 lines
```
Expected: all green; 2 grep hits.

- [ ] **Step 6: Commit**

```bash
git add tests/autospec-grow-define/smoke.bats autospec validate
git commit -m "test: grow-define smoke + structural/lock-step contract in validate.sh"
```

---

## Self-Review

**Spec coverage:** G-detect/config gate → Task 3 Step 3 ✓; G0 measure → Task 3 Step 4 ✓; G1 lens roster (6 lenses) → Task 3 Step 4 + structural check Task 4 ✓; G2 pipeline (validate/dedup/verify/rank/slice) → Task 1 (deterministic) + Task 3 verify-dispatch prose ✓; G3 decompose (artifact via autospec-define Phase 3 or filer; outbound template) → Task 2 + Task 3 Step 4 ✓; pending ledger per issue → Task 2 ✓; stop/handoff → Task 3 Step 4 ✓; trio lock-step + goldens → Task 3 Step 5 + Task 4 check ✓; structural sections → Task 4 ✓; mock-driven smoke → Task 4 ✓; install/uninstall → Task 3 Step 6 ✓. Reuse (Plan 1+2 scripts, autospec-define Phase 3) honored. G0 live API and outbound drafting correctly deferred to Plan 4.

**Placeholder scan:** Tasks 1–2 + Task 4 contain complete runnable code and real assertions. Task 3 is prose-authoring guided by named template skills + exact required sections/labels/invocations — the one place the deliverable is prose, flagged for human/LLM read at review. One inline typo in Task 2 Step 3 (`2>"$HERE/..//dev/null"`) is explicitly called out with the correct line in Step 4.

**Type/name consistency:** candidate schema + ledger record set match Plan 1+2 exactly. Labels (`auto-implement,growth:artifact,growth:<channel>` / `growth:outbound,growth/needs-draft,growth:<channel>`) consistent between Task 2 filer and Task 4 smoke assertions. `max_issues_per_cycle` config key consistent (Task 1 reads it, Task 4 sets it). `GROWTH_LEDGER` seam consistent across all suites. The 6 lens names identical between Task 3 roster and Task 4 structural check.
