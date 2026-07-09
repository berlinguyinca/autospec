# AutoSpec Growth — Candidate Pipeline (Plan 2 of 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the deterministic growth candidate pipeline — candidate schema + validator, dedup-against-ledger (full seen-set), source-weighted severity-first ranker, and a fail-closed verify-harness — each backed by a bats contract wired into `validate.sh`.

**Architecture:** Four bash+jq primitives under `skills/autospec-shared/scripts/` plus a JSON Schema under `skills/autospec-shared/schemas/`, consuming Plan 1's `growth-ledger.sh` (append `refuted`, read seen-set) and `growth-source-weights.sh` (per-lens ranking weight). No new runtime, no live API calls. The final task registers `check_growth_candidate_pipeline_contract` in `validate.sh`.

**Tech Stack:** bash (macOS bash 3.2 compatible), `jq`, bats-core, JSON Schema validated via `jq` (no external validator).

## Global Constraints

- **Bash 3.2 compatible.** No associative arrays, no `${var,,}`; use `tr`/`awk`/`case`. In bats, write fixtures to a real temp file before any `[ -f ]` check (`[ -f <(...) ]` is false on macOS bash 3.2).
- **`set -euo pipefail`** at the top of every script. One-sided conditionals use `if/then/fi`, never `[ test ] && action`. No `RETURN` traps (use inline cleanup).
- **Fail-closed:** missing input files, malformed JSON, or an unparseable verify verdict → exit non-zero / treat as refuted. Never silently emit empty on error.
- **String equality only** for `norm_title` matching — never `grep` regex / `jq test()` on candidate-derived values (metacharacter-injection hazard). Use `jq`'s `==` / `index`.
- **The 6 lens names** (exact): `technical-seo`, `keyword-gap`, `content-opportunity`, `community`, `directory`, `backlink`. **kind** ∈ `artifact|outbound`. **channel** ∈ `technical_seo|content|outreach|directories`.
- **Ledger record schema** (from Plan 1, do not change): `{round, source, title, norm_title, channel, kind, issue, outcome, reason, ts}`; `outcome` ∈ `pending|merged_clean|published|rejected|refuted|failed`; readers take latest line per issue, `issue:0` lines kept individually.
- **Env seams:** `GROWTH_LEDGER` (ledger path). Tests set it to a temp file and build ledger state via the real `growth-ledger.sh --append` — never a bespoke fixture that could mask a schema mismatch.
- **Conventional commits**, branch `feat/autospec-growth-candidate-pipeline`, never `--no-verify`, never push to `main`. New scripts are `bash -n` clean and `chmod +x`.

---

### Task 1: Candidate schema + validator

**Files:**
- Create: `skills/autospec-shared/schemas/growth-candidate.schema.json`
- Create: `skills/autospec-shared/scripts/validate-growth-candidate.sh`
- Test: `tests/unit/growth-candidate-validate.bats`

**Interfaces:**
- Consumes: nothing.
- Produces: `validate-growth-candidate.sh <candidate.json>` — exit 0 valid, non-zero + stderr reason invalid. Required fields + constraints: `lens` ∈ the 6 names; `channel` ∈ the 4 channels; `kind` ∈ {artifact,outbound}; `title` non-empty string; `norm_title` non-empty string; `roi` integer 1..5; `severity` integer 1..5; `effort` ∈ {small,medium,large}; `confidence` number 0..1. `rationale`/`evidence` optional.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-candidate-validate.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-growth-candidate.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid() {
  cat > "$TMP/c.json" <<'JSON'
{"lens":"keyword-gap","channel":"content","kind":"artifact",
 "title":"Add vs page","norm_title":"add vs page",
 "rationale":"gsc pos 12","evidence":["gsc:q=x vs y"],
 "roi":4,"effort":"medium","severity":3,"confidence":0.7}
JSON
  echo "$TMP/c.json"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a valid candidate" {
  run bash "$V" "$(valid)"; [ "$status" -eq 0 ]
}

@test "rejects unknown lens" {
  f="$(valid)"; jq '.lens="seo-wizard"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"lens"* ]]
}

@test "rejects roi out of range" {
  f="$(valid)"; jq '.roi=9' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"roi"* ]]
}

@test "rejects non-integer severity" {
  f="$(valid)"; jq '.severity=2.5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects bad effort" {
  f="$(valid)"; jq '.effort="huge"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"effort"* ]]
}

@test "rejects bad kind" {
  f="$(valid)"; jq '.kind="tweet"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"kind"* ]]
}

@test "rejects confidence above 1" {
  f="$(valid)"; jq '.confidence=1.5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects missing norm_title" {
  f="$(valid)"; jq 'del(.norm_title)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects malformed json" {
  printf 'not json {{{' > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-candidate-validate.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the schema (documentation artifact; validator is jq-driven)**

```json
// skills/autospec-shared/schemas/growth-candidate.schema.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "autospec growth candidate",
  "type": "object",
  "required": ["lens", "channel", "kind", "title", "norm_title", "roi", "effort", "severity", "confidence"],
  "properties": {
    "lens":       { "enum": ["technical-seo","keyword-gap","content-opportunity","community","directory","backlink"] },
    "channel":    { "enum": ["technical_seo","content","outreach","directories"] },
    "kind":       { "enum": ["artifact","outbound"] },
    "title":      { "type": "string", "minLength": 1 },
    "norm_title": { "type": "string", "minLength": 1 },
    "rationale":  { "type": "string" },
    "evidence":   { "type": "array", "items": { "type": "string" } },
    "roi":        { "type": "integer", "minimum": 1, "maximum": 5 },
    "severity":   { "type": "integer", "minimum": 1, "maximum": 5 },
    "effort":     { "enum": ["small","medium","large"] },
    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
  }
}
```

- [ ] **Step 4: Write the validator**

```bash
#!/usr/bin/env bash
# validate-growth-candidate.sh — validate one growth candidate JSON object.
# Usage: validate-growth-candidate.sh <candidate.json>   (exit 0 valid)
set -euo pipefail

C="${1:?usage: validate-growth-candidate.sh <candidate.json>}"
if [ ! -f "$C" ]; then echo "candidate not found: $C" >&2; exit 2; fi
if ! jq -e . "$C" >/dev/null 2>&1; then echo "not valid JSON: $C" >&2; exit 2; fi

fail() { echo "candidate invalid: $1" >&2; exit 1; }

jq -e '.lens as $l | ["technical-seo","keyword-gap","content-opportunity","community","directory","backlink"] | index($l)' "$C" >/dev/null \
  || fail "lens must be one of the 6 known lenses"
jq -e '.channel as $c | ["technical_seo","content","outreach","directories"] | index($c)' "$C" >/dev/null \
  || fail "channel must be one of technical_seo|content|outreach|directories"
jq -e '.kind as $k | ["artifact","outbound"] | index($k)' "$C" >/dev/null \
  || fail "kind must be artifact|outbound"
jq -e '.title // "" | length > 0' "$C" >/dev/null       || fail "title is required"
jq -e '.norm_title // "" | length > 0' "$C" >/dev/null   || fail "norm_title is required"
jq -e '.roi | numbers and . == floor and . >= 1 and . <= 5' "$C" >/dev/null \
  || fail "roi must be an integer 1..5"
jq -e '.severity | numbers and . == floor and . >= 1 and . <= 5' "$C" >/dev/null \
  || fail "severity must be an integer 1..5"
jq -e '.effort as $e | ["small","medium","large"] | index($e)' "$C" >/dev/null \
  || fail "effort must be small|medium|large"
jq -e '.confidence | numbers and . >= 0 and . <= 1' "$C" >/dev/null \
  || fail "confidence must be a number 0..1"

exit 0
```

- [ ] **Step 5: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/validate-growth-candidate.sh
bats tests/unit/growth-candidate-validate.bats
```
Expected: all PASS. (Note: `.roi | numbers` yields the number if `.roi` is numeric else `empty`; combined with `== floor` this rejects `2.5` and non-numbers, satisfying the integer tests.)

- [ ] **Step 6: Commit**

```bash
git add skills/autospec-shared/schemas/growth-candidate.schema.json \
        skills/autospec-shared/scripts/validate-growth-candidate.sh \
        tests/unit/growth-candidate-validate.bats
git commit -m "feat: growth candidate schema + validator"
```

---

### Task 2: Dedup-against-ledger

**Files:**
- Create: `skills/autospec-shared/scripts/growth-candidate-dedup.sh`
- Test: `tests/unit/growth-candidate-dedup.bats`

**Interfaces:**
- Consumes: the ledger's `norm_title` field (Plan 1 schema).
- Produces: `growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>` — prints (JSONL, one per line) the candidates whose `norm_title` does NOT appear on ANY ledger line (full seen-set, every outcome). Exact string match. Missing candidates file → non-zero; absent/empty ledger → all candidates pass through.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-candidate-dedup.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
D="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-dedup.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$3\",\"roi\":3,\"effort\":\"small\",\"severity\":3,\"confidence\":0.5}"; }
ledline() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"$2\",\"norm_title\":\"$3\",\"channel\":\"content\",\"kind\":\"artifact\",\"issue\":$4,\"outcome\":\"$5\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$D" ]; run bash -n "$D"; [ "$status" -eq 0 ]
}

@test "empty ledger: all candidates pass" {
  : > "$TMP/cands.jsonl"
  cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"
  cand community "B" "b" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c '"norm_title"')" -eq 2 ]
}

@test "drops candidate matching a merged_clean ledger line" {
  bash "$LG" --append "$(ledline keyword-gap "A" "a" 7 merged_clean)"
  : > "$TMP/cands.jsonl"; cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"; cand community "B" "b" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"b"'* ]]
  [[ "$output" != *'"norm_title":"a"'* ]]
}

@test "ALSO drops candidate matching a refuted ledger line (full seen-set)" {
  bash "$LG" --append "$(ledline community "B" "b" 0 refuted)"
  : > "$TMP/cands.jsonl"; cand community "B" "b" >> "$TMP/cands.jsonl"; cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"a"'* ]]
  [[ "$output" != *'"norm_title":"b"'* ]]
}

@test "missing candidates file fails" {
  run bash "$D" "$TMP/nope.jsonl" "$GROWTH_LEDGER"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-candidate-dedup.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the dedup script**

```bash
#!/usr/bin/env bash
# growth-candidate-dedup.sh — drop candidates whose norm_title already appears
# on ANY ledger line (full seen-set: pending/merged/published/rejected/refuted/failed).
set -euo pipefail

CANDS="${1:?usage: growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>}"
LEDGER="${2:?usage: growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>}"
if [ ! -f "$CANDS" ]; then echo "candidates not found: $CANDS" >&2; exit 2; fi

# Build the set of seen norm_titles (empty if ledger absent/empty).
seen='[]'
if [ -f "$LEDGER" ] && [ -s "$LEDGER" ]; then
  seen="$(jq -s '[.[].norm_title]' "$LEDGER")"
fi

# Emit candidates whose norm_title is not in the seen set (exact match via index).
jq -c --argjson seen "$seen" 'select(($seen | index(.norm_title)) | not)' "$CANDS"
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-candidate-dedup.sh
bats tests/unit/growth-candidate-dedup.bats
```
Expected: all PASS. (`jq -c 'select(...)'` over a JSONL file streams object-by-object; `index` on the seen array is exact string match, no regex.)

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-candidate-dedup.sh tests/unit/growth-candidate-dedup.bats
git commit -m "feat: growth candidate dedup against full ledger seen-set"
```

---

### Task 3: ROI/severity ranker

**Files:**
- Create: `skills/autospec-shared/scripts/growth-candidate-rank.sh`
- Test: `tests/unit/growth-candidate-rank.bats`

**Interfaces:**
- Consumes: `growth-source-weights.sh` (Plan 1) for per-lens weight.
- Produces: `growth-candidate-rank.sh <candidates.jsonl>` — prints candidates (JSONL) sorted descending by `rank_score` (attached as `.rank_score`). Formula: `rank_score = (roi/5*0.5 + severity/5*0.5) * confidence * effort_factor * source_weight`; `effort_factor` small 1.0 / medium 0.7 / large 0.4; `source_weight` from the weights JSON keyed by `lens` (default 0.5 if absent). Tiebreak on equal `rank_score`: higher `severity`, then higher `roi`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-candidate-rank.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
R="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-rank.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$2\",\"roi\":$3,\"effort\":\"$4\",\"severity\":$5,\"confidence\":$6}"; }

@test "script exists and is bash -n clean" {
  [ -f "$R" ]; run bash -n "$R"; [ "$status" -eq 0 ]
}

@test "higher roi/severity ranks first (empty ledger → equal source weights)" {
  : > "$TMP/c.jsonl"
  cand keyword-gap low 2 small 2 0.5 >> "$TMP/c.jsonl"
  cand keyword-gap high 5 small 5 0.9 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  [ "$status" -eq 0 ]
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"high"'* ]]
}

@test "effort_factor lowers a large-effort candidate below an equal small-effort one" {
  : > "$TMP/c.jsonl"
  cand keyword-gap big 4 large 4 0.8 >> "$TMP/c.jsonl"
  cand keyword-gap small 4 small 4 0.8 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"small"'* ]]
}

@test "severity-first tiebreak on equal rank_score" {
  # identical roi/effort/confidence/lens → same base; differ only in severity via equal score construction:
  # give equal rank_score by same fields but different severity+roi that produce equal (roi+severity) sum.
  : > "$TMP/c.jsonl"
  cand keyword-gap sevhi 2 small 5 0.5 >> "$TMP/c.jsonl"   # roi2 sev5 sum7
  cand keyword-gap sevlo 5 small 2 0.5 >> "$TMP/c.jsonl"   # roi5 sev2 sum7  → equal rank_score
  run bash "$R" "$TMP/c.jsonl"
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"sevhi"'* ]]
}

@test "attaches numeric rank_score" {
  : > "$TMP/c.jsonl"; cand keyword-gap a 3 small 3 0.6 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  echo "$output" | jq -e '.rank_score | numbers' >/dev/null
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-candidate-rank.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the ranker**

```bash
#!/usr/bin/env bash
# growth-candidate-rank.sh — attach a deterministic rank_score and sort desc.
set -euo pipefail

CANDS="${1:?usage: growth-candidate-rank.sh <candidates.jsonl>}"
if [ ! -f "$CANDS" ]; then echo "candidates not found: $CANDS" >&2; exit 2; fi

HERE="$(cd "$(dirname "$0")" && pwd)"
WEIGHTS="$("$HERE/growth-source-weights.sh" 2>/dev/null || echo '{}')"

jq -s --argjson w "$WEIGHTS" '
  def effort_factor: {"small":1.0,"medium":0.7,"large":0.4}[.effort] // 0.4;
  map(
    . as $c
    | ($w[$c.lens] // 0.5) as $sw
    | (($c.roi/5*0.5) + ($c.severity/5*0.5)) * $c.confidence * ($c|effort_factor) * $sw as $score
    | . + {rank_score: (($score*100000|round)/100000)}
  )
  | sort_by([-.rank_score, -.severity, -.roi])
  | .[]
' "$CANDS"
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-candidate-rank.sh
bats tests/unit/growth-candidate-rank.bats
```
Expected: all PASS. (`sort_by` ascending on negated keys = descending; the array `[-.rank_score, -.severity, -.roi]` gives the severity-first, then roi tiebreak.)

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-candidate-rank.sh tests/unit/growth-candidate-rank.bats
git commit -m "feat: source-weighted severity-first candidate ranker"
```

---

### Task 4: Verify-harness

**Files:**
- Create: `skills/autospec-shared/scripts/growth-candidate-verify.sh`
- Test: `tests/unit/growth-candidate-verify.bats`

**Interfaces:**
- Consumes: `growth-ledger.sh --append` (Plan 1) to record refutations.
- Produces: `growth-candidate-verify.sh <candidate.json> <verdict.json>`. Verdict `{"real":bool,"reason":str}`. `real==true` → print the candidate unchanged (stdout), no ledger write. `real==false` → append one `refuted` ledger line (`issue:0`, `source:<lens>`, `outcome:"refuted"`, `reason` from verdict) and print nothing. Fail-closed: missing/unparseable verdict or `real` not boolean → treat as refuted with reason `"unparseable verdict, refused"`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-candidate-verify.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
VF="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-verify.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { cat > "$TMP/c.json" <<'JSON'
{"lens":"community","channel":"outreach","kind":"outbound","title":"post","norm_title":"post","roi":3,"effort":"small","severity":3,"confidence":0.6}
JSON
echo "$TMP/c.json"; }

@test "script exists and is bash -n clean" {
  [ -f "$VF" ]; run bash -n "$VF"; [ "$status" -eq 0 ]
}

@test "real:true emits the candidate and writes no ledger line" {
  echo '{"real":true,"reason":"ok"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"norm_title":"post"'* ]]
  [ ! -f "$GROWTH_LEDGER" ] || [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 0 ]
}

@test "real:false emits nothing and writes exactly one refuted line" {
  echo '{"real":false,"reason":"off-topic"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
  run bash "$LG" --show --json
  [[ "$output" == *'"outcome":"refuted"'* ]]
  [[ "$output" == *'off-topic'* ]]
}

@test "unparseable verdict fails closed (refuted)" {
  printf 'not json' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
  run bash "$LG" --show --json
  [[ "$output" == *'refuted'* ]]
  [[ "$output" == *'unparseable verdict'* ]]
}

@test "non-boolean real fails closed (refuted)" {
  echo '{"real":"yes"}' > "$TMP/v.json"
  run bash "$VF" "$(cand)" "$TMP/v.json"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 1 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-candidate-verify.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the verify-harness**

```bash
#!/usr/bin/env bash
# growth-candidate-verify.sh — record an LLM adversarial-verify verdict.
# real:true  -> emit candidate; real:false or unparseable -> refute (ledger).
set -euo pipefail

CAND="${1:?usage: growth-candidate-verify.sh <candidate.json> <verdict.json>}"
VERDICT="${2:?usage: growth-candidate-verify.sh <candidate.json> <verdict.json>}"
if [ ! -f "$CAND" ]; then echo "candidate not found: $CAND" >&2; exit 2; fi

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"

refute() {
  local reason="$1"
  local lens title norm channel kind
  lens="$(jq -r '.lens // "unknown"' "$CAND")"
  title="$(jq -r '.title // ""' "$CAND")"
  norm="$(jq -r '.norm_title // ""' "$CAND")"
  channel="$(jq -r '.channel // ""' "$CAND")"
  kind="$(jq -r '.kind // "outbound"' "$CAND")"
  local line
  line="$(jq -n --arg s "$lens" --arg t "$title" --arg n "$norm" \
             --arg c "$channel" --arg k "$kind" --arg r "$reason" \
    '{round:1,source:$s,title:$t,norm_title:$n,channel:$c,kind:$k,issue:0,outcome:"refuted",reason:$r,ts:"1970-01-01T00:00:00Z"}')"
  "$LEDGER_SH" --append "$line"
}

# Fail-closed: unparseable verdict or non-boolean .real -> refute.
if [ ! -f "$VERDICT" ] || ! jq -e 'has("real") and (.real|type=="boolean")' "$VERDICT" >/dev/null 2>&1; then
  refute "unparseable verdict, refused"
  exit 0
fi

if jq -e '.real == true' "$VERDICT" >/dev/null; then
  jq -c . "$CAND"
else
  reason="$(jq -r '.reason // "refuted"' "$VERDICT")"
  refute "$reason"
fi
exit 0
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-candidate-verify.sh
bats tests/unit/growth-candidate-verify.bats
```
Expected: all PASS. Note: the refuted ledger line uses a fixed epoch-free ISO `ts` placeholder (`1970-...`) because `growth-ledger.sh --append` requires a `ts` and this script has no clock seam; the ledger only uses `ts` for cadence (published lines), so a placeholder on refutations is inert.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-candidate-verify.sh tests/unit/growth-candidate-verify.bats
git commit -m "feat: fail-closed candidate verify-harness (records refutations)"
```

---

### Task 5: Integration test + `validate.sh` wiring

**Files:**
- Create: `tests/unit/growth-candidate-pipeline-integration.bats`
- Modify: `scripts/validate.sh` (add `check_growth_candidate_pipeline_contract` + register it)

**Interfaces:**
- Consumes: all four scripts from Tasks 1–4 + `growth-ledger.sh`.
- Produces: an end-to-end test (validate → dedup → verify → rank) and a `check_growth_candidate_pipeline_contract` that fails the run if any pipeline script is not `bash -n` clean or any pipeline bats suite fails.

- [ ] **Step 1: Write the integration test**

```bash
# tests/unit/growth-candidate-pipeline-integration.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
S="$REPO_ROOT/skills/autospec-shared/scripts"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$2\",\"roi\":$3,\"effort\":\"small\",\"severity\":$4,\"confidence\":0.8}"; }
ledline() { echo "{\"round\":1,\"source\":\"keyword-gap\",\"title\":\"seen\",\"norm_title\":\"seen\",\"channel\":\"content\",\"kind\":\"artifact\",\"issue\":5,\"outcome\":\"merged_clean\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "validate -> dedup -> verify -> rank end-to-end" {
  # a previously-shipped item is in the ledger and must be deduped out
  bash "$S/growth-ledger.sh" --append "$(ledline)"

  : > "$TMP/cands.jsonl"
  cand keyword-gap seen 5 5 >> "$TMP/cands.jsonl"   # duplicate of ledger -> dropped
  cand keyword-gap alpha 5 5 >> "$TMP/cands.jsonl"  # strong
  cand community beta 2 2 >> "$TMP/cands.jsonl"     # weak

  # 1. validate each candidate
  while IFS= read -r line; do
    echo "$line" > "$TMP/one.json"
    bash "$S/validate-growth-candidate.sh" "$TMP/one.json"
  done < "$TMP/cands.jsonl"

  # 2. dedup against ledger
  bash "$S/growth-candidate-dedup.sh" "$TMP/cands.jsonl" "$GROWTH_LEDGER" > "$TMP/deduped.jsonl"
  [ "$(grep -c '"norm_title"' "$TMP/deduped.jsonl")" -eq 2 ]
  ! grep -q '"norm_title":"seen"' "$TMP/deduped.jsonl"

  # 3. verify each survivor (all real:true here) -> collect
  echo '{"real":true,"reason":"ok"}' > "$TMP/v.json"
  : > "$TMP/verified.jsonl"
  while IFS= read -r line; do
    echo "$line" > "$TMP/one.json"
    bash "$S/growth-candidate-verify.sh" "$TMP/one.json" "$TMP/v.json" >> "$TMP/verified.jsonl"
  done < "$TMP/deduped.jsonl"

  # 4. rank -> alpha (roi5/sev5) must be first, beta last
  run bash "$S/growth-candidate-rank.sh" "$TMP/verified.jsonl"
  [ "$status" -eq 0 ]
  first="$(printf '%s\n' "$output" | head -1)"
  last="$(printf '%s\n' "$output" | tail -1)"
  [[ "$first" == *'"title":"alpha"'* ]]
  [[ "$last" == *'"title":"beta"'* ]]
}
```

- [ ] **Step 2: Run integration test to verify it passes**

Run: `bats tests/unit/growth-candidate-pipeline-integration.bats`
Expected: PASS (Tasks 1–4 already implemented). If it fails, the defect is in one of the pipeline scripts — fix there, do not weaken the test.

- [ ] **Step 3: Add the validate.sh check function**

Locate the idiom first: `grep -n "check_growth_shared_contract" scripts/validate.sh`. Add `check_growth_candidate_pipeline_contract` immediately after `check_growth_shared_contract`'s definition, matching its structure exactly:

```bash
check_growth_candidate_pipeline_contract() {
    info "growth candidate pipeline: bash -n + tests/unit/growth-candidate-*.bats"
    local s
    for s in validate-growth-candidate growth-candidate-dedup growth-candidate-rank growth-candidate-verify; do
        check_bash_syntax "skills/autospec-shared/scripts/$s.sh"
        [ -f "skills/autospec-shared/scripts/$s.sh" ] \
            || fail "skills/autospec-shared/scripts/$s.sh: required candidate-pipeline script missing"
    done
    if ! command -v bats >/dev/null 2>&1; then
        info "  bats not available — skipping candidate-pipeline bats suites"
        return 0
    fi
    local t
    for t in tests/unit/growth-candidate-validate.bats tests/unit/growth-candidate-dedup.bats \
             tests/unit/growth-candidate-rank.bats tests/unit/growth-candidate-verify.bats \
             tests/unit/growth-candidate-pipeline-integration.bats; do
        [ -f "$t" ] || fail "$t: candidate-pipeline bats coverage missing"
        info "  running: $t"
        bats "$t" >/tmp/validate-growth-candidate-suite.log 2>&1 \
            || { cat /tmp/validate-growth-candidate-suite.log >&2; \
                 fail "$t: failed (candidate-pipeline contract)"; }
    done
}
```

- [ ] **Step 4: Register the check in `main`'s run list**

Add `check_growth_candidate_pipeline_contract` on the line immediately after `check_growth_shared_contract` in `main`'s run list (find it: `grep -n "check_growth_shared_contract" scripts/validate.sh` — the occurrence inside the run list, not the function definition).

- [ ] **Step 5: Verify the check and the full suite**

```bash
bash -n scripts/validate.sh
# run the new suites directly to confirm green:
bats tests/unit/growth-candidate-validate.bats tests/unit/growth-candidate-dedup.bats \
     tests/unit/growth-candidate-rank.bats tests/unit/growth-candidate-verify.bats \
     tests/unit/growth-candidate-pipeline-integration.bats
# full suite (slow) — run in a clean checkout you are not mutating:
bash scripts/validate.sh
```
Expected: new suites all pass; `bash scripts/validate.sh` ends `OK — all validation checks passed`. **Do not switch/delete branches while this background run is in flight** (it corrupts the working tree mid-run).

- [ ] **Step 6: Commit**

```bash
git add tests/unit/growth-candidate-pipeline-integration.bats scripts/validate.sh
git commit -m "test: wire growth candidate-pipeline contract into validate.sh"
```

---

## Self-Review

**Spec coverage:** candidate schema+validator → Task 1 ✓; dedup-against-full-seen-set → Task 2 ✓; source-weighted severity-first ranker → Task 3 ✓; fail-closed verify-harness recording refutations → Task 4 ✓; `validate.sh` wiring → Task 5 ✓; end-to-end integration built on the real `growth-ledger.sh --append` (anti-self-consistent-fixture) → Task 5 integration test ✓. Lens prompts / top-N cutoff explicitly out of scope (Plan 3) per the spec. No spec requirement left without a task.

**Placeholder scan:** every step contains complete, runnable code + real assertions. No TBD/TODO/"handle edge cases".

**Type/name consistency:** candidate field set `{lens, channel, kind, title, norm_title, rationale?, evidence?, roi, effort, severity, confidence}` identical across Tasks 1–4 fixtures and the ranker jq. Lens names, `kind`, `channel` enums identical to Global Constraints. Ledger append in Task 4 uses the Plan 1 required-key set `{round,source,title,norm_title,channel,kind,issue,outcome,reason,ts}` with `issue:0`+`outcome:"refuted"`, matching Plan 1's `do_stats` refuted handling. `rank_score` produced in Task 3 is read nowhere else (terminal field). `GROWTH_LEDGER` seam consistent across all suites.
