# /autospec-grow-run Implementation Plan (Plan 4 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `/autospec-grow-run` — the produce→gate→publish→measure half of the growth loop: drain `growth:artifact` issues through `/autospec-run` with a content-quality gate, run the `growth:outbound` draft→ethics→queue pipeline (package-only, never auto-post), and measure/attribute via four live adapters that feed per-lens re-weighting.

**Architecture:** A lock-step trio skill (`skills/autospec-grow-run/`) orchestrates a new deterministic spine of bash/jq primitives under `skills/autospec-shared/scripts/`. Reuses Plan 1 ethics/measure/ledger/source-weights and Plan 2 candidate primitives; invokes `/autospec-run`, `autospec-review/secaudit/persona` rather than forking. All new scripts are `set -euo pipefail`, bash 3.2 compatible, fail-closed, with bats suites under `tests/unit/`.

**Tech Stack:** POSIX/bash 3.2, `jq`, `gh`, `curl` (behind a `GROWTH_FETCH_CMD` seam), the autospec trio-skill toolchain (`derive-trio.sh`, `gen-skill-goldens.sh`, `validate.sh`).

## Global Constraints

- Every new `.sh` starts with `#!/usr/bin/env bash` then `set -euo pipefail`; bash 3.2 compatible (no associative arrays, no `${var,,}`, no RETURN traps; one-sided conditionals use `if/then/fi`, never `[ test ] && action`).
- Fail-closed everywhere: missing/malformed input → non-zero + stderr reason; never silently empty on error.
- Deterministic time via `now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }` — copy this seam verbatim; tests set `GROWTH_NOW_EPOCH`.
- Deterministic ledger path via the `GROWTH_LEDGER` env seam where a script writes the ledger; reuse `growth-ledger.sh --append` for all writes (never hand-format ledger lines).
- String equality for any platform/label/title match — never `jq test()` on host/user-derived values (regex-metachar injection); use `==` or `capture()`+`==`.
- The agent NEVER posts to a third-party platform. Outbound publishing is package-only; a `published` ledger line is written only after a human confirms the live URL.
- Reuse over fork (ROI-check): cadence → `growth-ethics-precheck.sh --cadence`; FTC disclosure → `growth-ethics-precheck.sh --disclosure`; normalization → `growth-measure.sh --normalize`; re-weighting → `growth-source-weights.sh`; ledger → `growth-ledger.sh`.
- New shared scripts install via `SHARED_LIB_SCRIPT_FILES` (resolve_shared_lib_scripts_dir → `skills/autospec-shared/scripts/`), NOT the repo-root `SHARED_SCRIPT_FILES` group. (The Plan 3 install bug.)
- Editing `SKILL.md` requires re-deriving the trio (`derive-trio.sh <skill> --in-place`) AND regenerating goldens (`gen-skill-goldens.sh <skill>`) in the SAME task, or `validate.sh` fails closed.
- Net-new bats suites must be enumerated in `validate.sh`'s `main` run list (gate-atomicity rule).

---

## File Structure

New files:
- `skills/autospec-shared/schemas/growth-outbound-draft.schema.json` — outbound draft record schema.
- `skills/autospec-shared/scripts/validate-outbound-draft.sh` — draft validator (fail-closed).
- `skills/autospec-shared/scripts/growth-content-quality-precheck.sh` — keyword-density + citation presence; delegates disclosure to the existing ethics precheck.
- `skills/autospec-shared/scripts/growth-outbound-queue.sh` — `--build-body` / `--read-state` for the control-repo approval issue.
- `skills/autospec-shared/scripts/growth-adapter-github.sh|-analytics.sh|-gsc.sh|-rank.sh` — four live adapters behind the fetch seam.
- `skills/autospec-shared/scripts/growth-attribute.sh` — per-lens contribution from metric deltas + ledger.
- `skills/autospec-grow-run/SKILL.md` (+ derived `codex/prompt.md`, `opencode/agent.md`) — the trio skill.
- `skills/autospec-grow-run/install.sh`, `uninstall.sh`.
- `tests/unit/*.bats` per script; `tests/autospec-grow-run/smoke.bats`.
- `tests/skill-goldens/autospec-grow-run.sha256` (generated).

Modified files:
- `skills/autospec-shared/scripts/growth-measure.sh` — add `analytics` + `rank` normalize cases (+ its bats).
- `autospec validate` — add `check_grow_run_pipeline_contract` + `check_grow_run_contract`, register in `main`.
- Root installer/uninstaller skill lists (register `autospec-grow-run`).

---

## Task 1: Outbound-draft schema + validator

**Files:**
- Create: `skills/autospec-shared/schemas/growth-outbound-draft.schema.json`
- Create: `skills/autospec-shared/scripts/validate-outbound-draft.sh`
- Test: `tests/unit/validate-outbound-draft.bats`

**Interfaces:**
- Produces: `validate-outbound-draft.sh <draft.json>` — exit 0 valid, non-zero + stderr reason invalid. Draft record: `{issue:int, platform:str, target_url:str, body:str(non-empty), self_promo_rule:str, disclosure?:str, evidence:[str,...]}`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/validate-outbound-draft.bats
setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/validate-outbound-draft.sh"
  TMP="$(mktemp -d)"
}
teardown() { rm -rf "$TMP"; }

valid_draft() {
  cat > "$TMP/d.json" <<'JSON'
{"issue":42,"platform":"reddit","target_url":"https://reddit.com/r/x","body":"Hello, useful post.","self_promo_rule":"r/x allows tool posts on Saturdays","evidence":["gsc:q=x"]}
JSON
}

@test "valid draft passes" { valid_draft; run bash "$SCRIPT" "$TMP/d.json"; [ "$status" -eq 0 ]; }
@test "empty body rejected" { valid_draft; jq '.body=""' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "missing self_promo_rule rejected" { valid_draft; jq 'del(.self_promo_rule)' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "non-int issue rejected" { valid_draft; jq '.issue="42"' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "evidence not array rejected" { valid_draft; jq '.evidence="x"' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "malformed json rejected" { echo '{not json' > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "missing file rejected" { run bash "$SCRIPT" "$TMP/nope.json"; [ "$status" -ne 0 ]; }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/validate-outbound-draft.bats`
Expected: FAIL (script not found).

- [ ] **Step 3: Write the schema**

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "growth-outbound-draft",
  "type": "object",
  "required": ["issue", "platform", "target_url", "body", "self_promo_rule", "evidence"],
  "properties": {
    "issue": { "type": "integer" },
    "platform": { "type": "string", "minLength": 1 },
    "target_url": { "type": "string", "minLength": 1 },
    "body": { "type": "string", "minLength": 1 },
    "self_promo_rule": { "type": "string", "minLength": 1 },
    "disclosure": { "type": "string" },
    "evidence": { "type": "array", "items": { "type": "string" } }
  },
  "additionalProperties": true
}
```

- [ ] **Step 4: Write the validator**

```bash
#!/usr/bin/env bash
# validate-outbound-draft.sh — validate an outbound draft record. Fail-closed.
set -euo pipefail

draft="${1:-}"
[ -n "$draft" ] || { echo "usage: validate-outbound-draft.sh <draft.json>" >&2; exit 2; }
[ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
if ! jq . "$draft" >/dev/null 2>&1; then
  echo "malformed json: $draft" >&2; exit 1
fi

# Emit the first failing reason, exit 1. Types checked explicitly (a numeric
# "issue" as a string must fail; a non-array evidence must fail).
reason="$(jq -r '
  if (.issue|type) != "number" or ((.issue|floor) != .issue) then "issue must be an integer"
  elif (.platform|type) != "string" or (.platform|length)==0 then "platform must be a non-empty string"
  elif (.target_url|type) != "string" or (.target_url|length)==0 then "target_url must be a non-empty string"
  elif (.body|type) != "string" or (.body|length)==0 then "body must be a non-empty string"
  elif (.self_promo_rule|type) != "string" or (.self_promo_rule|length)==0 then "self_promo_rule must be a non-empty string"
  elif (has("disclosure") and (.disclosure|type)!="string") then "disclosure must be a string when present"
  elif (.evidence|type) != "array" then "evidence must be an array"
  elif ([.evidence[]|select(type!="string")]|length) > 0 then "evidence items must be strings"
  else "" end' "$draft")"

if [ -n "$reason" ]; then
  echo "invalid draft: $reason" >&2; exit 1
fi
exit 0
```

- [ ] **Step 5: Run test to verify it passes**

Run: `bats tests/unit/validate-outbound-draft.bats`
Expected: PASS (7/7).

- [ ] **Step 6: Commit**

```bash
git add skills/autospec-shared/schemas/growth-outbound-draft.schema.json skills/autospec-shared/scripts/validate-outbound-draft.sh tests/unit/validate-outbound-draft.bats
git commit -m "feat(grow-run): outbound-draft schema + validator"
```

---

## Task 2: Content-quality pre-check

**Files:**
- Create: `skills/autospec-shared/scripts/growth-content-quality-precheck.sh`
- Test: `tests/unit/growth-content-quality-precheck.bats`

**Interfaces:**
- Consumes: `growth-ethics-precheck.sh --disclosure <file>` (Plan 1) for the FTC check.
- Produces: `growth-content-quality-precheck.sh <content.md>` — deterministic pre-checks feeding the R1 content-quality gate. Exit 0 = clean; non-zero + stderr reason on a hard failure. Env: `GROWTH_KW_DENSITY_MAX` (default `0.06`), `GROWTH_MIN_CITATIONS` (default `0`; the skill raises it for pages that make factual claims). Delegates disclosure to the ethics precheck; if that script is absent, the disclosure sub-check is skipped (logged) — density/citation still run.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-content-quality-precheck.bats
setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-content-quality-precheck.sh"
  TMP="$(mktemp -d)"
}
teardown() { rm -rf "$TMP"; }

@test "clean prose passes" {
  printf 'AutoSpec ships specs. It helps teams plan work and review changes carefully.\n' > "$TMP/c.md"
  run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -eq 0 ]
}
@test "keyword-stuffed content fails density" {
  # 'growth' 8 of ~10 words -> density 0.8 > 0.06
  printf 'growth growth growth growth growth growth growth growth tool now\n' > "$TMP/c.md"
  run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -ne 0 ]
  [[ "$output" == *density* ]]
}
@test "missing citation fails when GROWTH_MIN_CITATIONS=1" {
  printf 'A plain sentence with no links at all here.\n' > "$TMP/c.md"
  GROWTH_MIN_CITATIONS=1 run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -ne 0 ]
  [[ "$output" == *citation* ]]
}
@test "http link satisfies citation requirement" {
  printf 'See the benchmark at https://example.com/bench for details here.\n' > "$TMP/c.md"
  GROWTH_MIN_CITATIONS=1 run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -eq 0 ]
}
@test "missing file rejected" { run bash "$SCRIPT" "$TMP/nope.md"; [ "$status" -ne 0 ]; }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-content-quality-precheck.bats`
Expected: FAIL (script not found).

- [ ] **Step 3: Write the pre-check**

```bash
#!/usr/bin/env bash
# growth-content-quality-precheck.sh — deterministic pre-checks feeding the
# content-quality gate: keyword-density ceiling + citation presence. Delegates
# FTC disclosure to growth-ethics-precheck.sh. Fail-closed.
set -euo pipefail

content="${1:-}"
[ -n "$content" ] || { echo "usage: growth-content-quality-precheck.sh <content.md>" >&2; exit 2; }
[ -f "$content" ] || { echo "content not found: $content" >&2; exit 2; }

KW_MAX="${GROWTH_KW_DENSITY_MAX:-0.06}"
MIN_CITATIONS="${GROWTH_MIN_CITATIONS:-0}"

# --- keyword density: max single-token share over total tokens ---------------
# Lowercase, split on non-alphanumeric, drop empties, count most frequent / total.
density="$(tr '[:upper:]' '[:lower:]' < "$content" \
  | tr -c 'a-z0-9' '\n' \
  | grep -v '^$' \
  | awk '{c[$0]++; n++} END { if (n==0){print 0; exit} m=0; for (k in c) if (c[k]>m) m=c[k]; printf "%.6f", m/n }')"

# Compare density > KW_MAX using awk (no bc dependency).
if awk -v d="$density" -v m="$KW_MAX" 'BEGIN{ exit !(d > m) }'; then
  echo "content-quality: keyword density $density exceeds max $KW_MAX (stuffing)" >&2
  exit 1
fi

# --- citation presence -------------------------------------------------------
if [ "$MIN_CITATIONS" -gt 0 ]; then
  links="$(grep -Eoc 'https?://[^[:space:])]+' "$content" || true)"
  [ -n "$links" ] || links=0
  if [ "$links" -lt "$MIN_CITATIONS" ]; then
    echo "content-quality: $links citation(s) found, need >= $MIN_CITATIONS" >&2
    exit 1
  fi
fi

# --- disclosure (delegate; skip if the Plan 1 script is unavailable) ---------
here="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$here/growth-ethics-precheck.sh" ]; then
  bash "$here/growth-ethics-precheck.sh" --disclosure "$content"
else
  echo "content-quality: growth-ethics-precheck.sh not found, skipping disclosure check" >&2
fi

exit 0
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/unit/growth-content-quality-precheck.bats`
Expected: PASS (5/5).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-content-quality-precheck.sh tests/unit/growth-content-quality-precheck.bats
git commit -m "feat(grow-run): content-quality pre-check (density + citations, delegates disclosure)"
```

---

## Task 3: Outbound approval-queue helper

**Files:**
- Create: `skills/autospec-shared/scripts/growth-outbound-queue.sh`
- Test: `tests/unit/growth-outbound-queue.bats`

**Interfaces:**
- Consumes: a validated draft JSON (Task 1 shape).
- Produces:
  - `growth-outbound-queue.sh --build-body <draft.json>` — prints the ready-to-paste `growth/needs-approval` issue body (markdown) to stdout: the exact content, target link, rationale, and the self-promo rule satisfied. No `gh` calls (the skill shells `gh`).
  - `growth-outbound-queue.sh --read-state <label-csv>` — prints exactly one of `approved|rejected|edited|pending` from a comma-separated label list, by string equality (never regex). Precedence: `growth/rejected` > `growth/approved` > `growth/edited` > else `pending`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-outbound-queue.bats
setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-outbound-queue.sh"
  TMP="$(mktemp -d)"
  cat > "$TMP/d.json" <<'JSON'
{"issue":42,"platform":"reddit","target_url":"https://reddit.com/r/x","body":"Hello.","self_promo_rule":"Saturdays only","evidence":["gsc:q=x"]}
JSON
}
teardown() { rm -rf "$TMP"; }

@test "build-body includes target, rule, and body" {
  run bash "$SCRIPT" --build-body "$TMP/d.json"; [ "$status" -eq 0 ]
  [[ "$output" == *"https://reddit.com/r/x"* ]]
  [[ "$output" == *"Saturdays only"* ]]
  [[ "$output" == *"Hello."* ]]
}
@test "read-state approved" { run bash "$SCRIPT" --read-state "growth:outbound,growth/approved"; [ "$status" -eq 0 ]; [ "$output" = "approved" ]; }
@test "read-state rejected wins over approved" { run bash "$SCRIPT" --read-state "growth/approved,growth/rejected"; [ "$output" = "rejected" ]; }
@test "read-state edited" { run bash "$SCRIPT" --read-state "growth/edited"; [ "$output" = "edited" ]; }
@test "read-state default pending" { run bash "$SCRIPT" --read-state "growth:outbound"; [ "$output" = "pending" ]; }
@test "read-state injection-safe: crafted label does not flip state" {
  run bash "$SCRIPT" --read-state "growth/approved-not-really,foo.*"; [ "$output" = "pending" ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-outbound-queue.bats`
Expected: FAIL (script not found).

- [ ] **Step 3: Write the helper**

```bash
#!/usr/bin/env bash
# growth-outbound-queue.sh — deterministic body/label logic for the outbound
# approval control issue. No gh calls (the skill shells gh). Fail-closed.
set -euo pipefail

build_body() {
  local draft="${1:?draft required}"
  [ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
  jq . "$draft" >/dev/null 2>&1 || { echo "malformed draft: $draft" >&2; exit 1; }
  # Render a stable markdown body. jq -r pulls fields; missing -> fail-closed.
  jq -r '
    "## Outbound draft ready for approval",
    "",
    "**Source issue:** #\(.issue)",
    "**Platform:** \(.platform)",
    "**Target:** \(.target_url)",
    "**Self-promo rule satisfied:** \(.self_promo_rule)",
    (if (.disclosure? // "") != "" then "**Disclosure:** \(.disclosure)" else empty end),
    "",
    "### Ready-to-paste content",
    "",
    .body,
    "",
    "### Evidence",
    (.evidence[] | "- \(.)"),
    "",
    "---",
    "Apply `growth/approved` to receive the one-click package, edit this body to re-gate, or `growth/rejected` to drop.  The agent never posts on your behalf."
  ' "$draft"
}

read_state() {
  local csv="${1:-}"
  # Split on commas; exact match each token (string equality, never regex).
  local IFS=','; local tok; local approved=0 rejected=0 edited=0
  for tok in $csv; do
    case "$tok" in
      "growth/rejected") rejected=1 ;;
      "growth/approved") approved=1 ;;
      "growth/edited")   edited=1 ;;
    esac
  done
  if [ "$rejected" -eq 1 ]; then echo "rejected"
  elif [ "$approved" -eq 1 ]; then echo "approved"
  elif [ "$edited" -eq 1 ]; then echo "edited"
  else echo "pending"; fi
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --build-body) build_body "$@" ;;
  --read-state) read_state "$@" ;;
  *) echo "usage: growth-outbound-queue.sh --build-body <draft.json> | --read-state <label-csv>" >&2; exit 2 ;;
esac
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/unit/growth-outbound-queue.bats`
Expected: PASS (6/6).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-outbound-queue.sh tests/unit/growth-outbound-queue.bats
git commit -m "feat(grow-run): outbound approval-queue body + label-state helper"
```

---

## Task 4: Extend the normalization seam (analytics + rank)

**Files:**
- Modify: `skills/autospec-shared/scripts/growth-measure.sh`
- Test: `tests/unit/growth-measure.bats` (extend existing; if none, create)

**Interfaces:**
- Produces: `growth-measure.sh --normalize analytics <raw.json>` → `{provider:"analytics", metrics:{visitors, pageviews}, ts}`; `growth-measure.sh --normalize rank <raw.json>` → `{provider:"rank", metrics:{avg_position, keywords:[{keyword,position}]}, ts}`. Existing `gsc`/`github` cases unchanged.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-measure.bats  (add these; keep any existing cases)
setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-measure.sh"
  TMP="$(mktemp -d)"
  export GROWTH_NOW_EPOCH=1000
}
teardown() { rm -rf "$TMP"; }

@test "normalize analytics (plausible-shaped)" {
  echo '{"results":{"visitors":{"value":120},"pageviews":{"value":540}}}' > "$TMP/a.json"
  run bash "$SCRIPT" --normalize analytics "$TMP/a.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "analytics" ]
  [ "$(echo "$output" | jq -r '.metrics.visitors')" = "120" ]
  [ "$(echo "$output" | jq -r '.metrics.pageviews')" = "540" ]
  [ "$(echo "$output" | jq -r '.ts')" = "1000" ]
}
@test "normalize rank" {
  echo '{"keywords":[{"keyword":"a","position":4},{"keyword":"b","position":12}]}' > "$TMP/r.json"
  run bash "$SCRIPT" --normalize rank "$TMP/r.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "rank" ]
  [ "$(echo "$output" | jq -r '.metrics.avg_position')" = "8" ]
}
@test "unknown provider still fails" {
  echo '{}' > "$TMP/u.json"; run bash "$SCRIPT" --normalize bogus "$TMP/u.json"; [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-measure.bats`
Expected: FAIL (analytics/rank unknown provider).

- [ ] **Step 3: Add the two cases**

In `growth-measure.sh`, inside the `case "$provider" in` block, before the `*)` arm, add:

```bash
    analytics)
      jq --argjson ts "$ts" '{
        provider: "analytics",
        metrics: {
          visitors: (.results.visitors.value // .visitors // 0),
          pageviews: (.results.pageviews.value // .pageviews // 0)
        },
        ts: $ts }' "$raw"
      ;;
    rank)
      jq --argjson ts "$ts" '{
        provider: "rank",
        metrics: {
          avg_position: ((([.keywords[]?.position] | add) // 0) / ([.keywords[]? ] | length | if . == 0 then 1 else . end)),
          keywords: [.keywords[]? | {keyword, position}]
        },
        ts: $ts }' "$raw"
      ;;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/unit/growth-measure.bats`
Expected: PASS (existing + 3 new).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-measure.sh tests/unit/growth-measure.bats
git commit -m "feat(grow-run): extend growth-measure normalize with analytics + rank"
```

---

## Task 5: Four live measurement adapters

**Files:**
- Create: `skills/autospec-shared/scripts/growth-adapter-github.sh`
- Create: `skills/autospec-shared/scripts/growth-adapter-analytics.sh`
- Create: `skills/autospec-shared/scripts/growth-adapter-gsc.sh`
- Create: `skills/autospec-shared/scripts/growth-adapter-rank.sh`
- Test: `tests/unit/growth-adapters.bats`

**Interfaces:**
- Consumes: `growth-measure.sh --normalize` (Task 4), config JSON.
- Produces: each `growth-adapter-<p>.sh <config.json>` fetches via the `GROWTH_FETCH_CMD` seam (default `curl -fsSL`) and prints the **normalized** envelope to stdout. Credentials read from the config-named env var. Missing cred env var → non-zero + empty (fail-closed). The fetch command receives the resolved URL as its final arg; tests set `GROWTH_FETCH_CMD` to a script that ignores the URL and prints a fixture.

Config paths read (permissive, with defaults):
- github: `.measurement.github.repo` (e.g. `owner/name`), token env `.measurement.github.token_env` (default `GITHUB_TOKEN`).
- analytics: `.measurement.analytics.provider` (`plausible`|`ga4`), site `.measurement.analytics.site`, token env `.measurement.analytics.token_env`.
- gsc: `.measurement.gsc.site`, token env `.measurement.gsc.token_env`.
- rank: `.measurement.rank.endpoint`, token env `.measurement.rank.token_env`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-adapters.bats
setup() {
  DIR="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts"
  TMP="$(mktemp -d)"
  export GROWTH_NOW_EPOCH=1000
  # Fake fetcher: ignore all args, print the fixture named by GROWTH_FIXTURE.
  cat > "$TMP/fetch.sh" <<'SH'
#!/usr/bin/env bash
cat "$GROWTH_FIXTURE"
SH
  chmod +x "$TMP/fetch.sh"
  export GROWTH_FETCH_CMD="$TMP/fetch.sh"
}
teardown() { rm -rf "$TMP"; }

@test "github adapter emits normalized github envelope" {
  echo '{"stargazers_count":10,"forks_count":3}' > "$TMP/gh.json"
  export GROWTH_FIXTURE="$TMP/gh.json" GITHUB_TOKEN=x
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"GITHUB_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "github" ]
  [ "$(echo "$output" | jq -r '.metrics.stars')" = "10" ]
}
@test "github adapter fails closed when token env unset" {
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"MISSING_TOK"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"; [ "$status" -ne 0 ]
  [ -z "$output" ]
}
@test "analytics adapter emits normalized envelope" {
  echo '{"results":{"visitors":{"value":9},"pageviews":{"value":20}}}' > "$TMP/a.json"
  export GROWTH_FIXTURE="$TMP/a.json" PLAUSIBLE_API_TOKEN=x
  echo '{"measurement":{"analytics":{"provider":"plausible","site":"x.com","token_env":"PLAUSIBLE_API_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-analytics.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.metrics.visitors')" = "9" ]
}
@test "gsc adapter emits normalized envelope" {
  echo '{"rows":[{"keys":["q"],"clicks":5,"impressions":50,"position":7}]}' > "$TMP/g.json"
  export GROWTH_FIXTURE="$TMP/g.json" GSC_TOKEN=x
  echo '{"measurement":{"gsc":{"site":"x.com","token_env":"GSC_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-gsc.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.metrics.clicks_total')" = "5" ]
}
@test "rank adapter emits normalized envelope" {
  echo '{"keywords":[{"keyword":"a","position":4}]}' > "$TMP/r.json"
  export GROWTH_FIXTURE="$TMP/r.json" RANK_TOKEN=x
  echo '{"measurement":{"rank":{"endpoint":"https://rank.example","token_env":"RANK_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-rank.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "rank" ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-adapters.bats`
Expected: FAIL (adapters not found).

- [ ] **Step 3: Write the four adapters**

Each adapter shares the same skeleton. `growth-adapter-github.sh`:

```bash
#!/usr/bin/env bash
# growth-adapter-github.sh — fetch GitHub repo signals, emit normalized envelope.
# Fetch goes through GROWTH_FETCH_CMD (default curl) so tests inject fixtures.
# Missing credentials -> non-zero + empty (fail-closed).
set -euo pipefail

cfg="${1:-}"
[ -n "$cfg" ] || { echo "usage: growth-adapter-github.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"

repo="$(jq -r '.measurement.github.repo // ""' "$cfg")"
tok_env="$(jq -r '.measurement.github.token_env // "GITHUB_TOKEN"' "$cfg")"
[ -n "$repo" ] || { echo "github adapter: .measurement.github.repo missing" >&2; exit 1; }
# Indirect env lookup, bash 3.2 safe.
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || { echo "github adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }

fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "https://api.github.com/repos/$repo")" || { echo "github adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize github "$tmp"
rm -f "$tmp"
```

`growth-adapter-analytics.sh` (same skeleton; provider-aware URL):

```bash
#!/usr/bin/env bash
# growth-adapter-analytics.sh — fetch analytics visitors/pageviews (Plausible or
# GA4), emit normalized envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-analytics.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
provider="$(jq -r '.measurement.analytics.provider // "plausible"' "$cfg")"
site="$(jq -r '.measurement.analytics.site // ""' "$cfg")"
tok_env="$(jq -r '.measurement.analytics.token_env // "PLAUSIBLE_API_TOKEN"' "$cfg")"
[ -n "$site" ] || { echo "analytics adapter: .measurement.analytics.site missing" >&2; exit 1; }
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || { echo "analytics adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }
case "$provider" in
  plausible) url="https://plausible.io/api/v1/stats/aggregate?site_id=$site&metrics=visitors,pageviews" ;;
  ga4)       url="https://analyticsdata.googleapis.com/v1beta/properties/$site:runReport" ;;
  *) echo "analytics adapter: unknown provider $provider" >&2; exit 1 ;;
esac
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$url")" || { echo "analytics adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize analytics "$tmp"
rm -f "$tmp"
```

`growth-adapter-gsc.sh`:

```bash
#!/usr/bin/env bash
# growth-adapter-gsc.sh — fetch Google Search Console rows, emit normalized
# envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-gsc.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
site="$(jq -r '.measurement.gsc.site // ""' "$cfg")"
tok_env="$(jq -r '.measurement.gsc.token_env // "GSC_TOKEN"' "$cfg")"
[ -n "$site" ] || { echo "gsc adapter: .measurement.gsc.site missing" >&2; exit 1; }
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || { echo "gsc adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }
url="https://searchconsole.googleapis.com/webmasters/v3/sites/$site/searchAnalytics/query"
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$url")" || { echo "gsc adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize gsc "$tmp"
rm -f "$tmp"
```

`growth-adapter-rank.sh`:

```bash
#!/usr/bin/env bash
# growth-adapter-rank.sh — fetch SERP rank/backlink data, emit normalized
# envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-rank.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
endpoint="$(jq -r '.measurement.rank.endpoint // ""' "$cfg")"
tok_env="$(jq -r '.measurement.rank.token_env // "RANK_TOKEN"' "$cfg")"
[ -n "$endpoint" ] || { echo "rank adapter: .measurement.rank.endpoint missing" >&2; exit 1; }
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || { echo "rank adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$endpoint")" || { echo "rank adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize rank "$tmp"
rm -f "$tmp"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/unit/growth-adapters.bats`
Expected: PASS (5/5).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-adapter-*.sh tests/unit/growth-adapters.bats
git commit -m "feat(grow-run): 4 live measurement adapters behind GROWTH_FETCH_CMD seam"
```

---

## Task 6: Attribution → per-lens contribution

**Files:**
- Create: `skills/autospec-shared/scripts/growth-attribute.sh`
- Test: `tests/unit/growth-attribute.bats`

**Interfaces:**
- Consumes: two normalized-envelope files (before/after) and the ledger JSONL.
- Produces: `growth-attribute.sh <before.json> <after.json> <ledger.jsonl>` — prints a per-lens contribution table `[{source, shipped, delta_share}]` to stdout. `delta_share` = the total positive metric delta divided evenly across lenses that shipped a `merged_clean`/`published` item in the window (a simple, deterministic v1 attribution; the ledger will inform a smarter model later). Empty ledger → `[]`. Fail-closed on malformed input.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-attribute.bats
setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-attribute.sh"
  TMP="$(mktemp -d)"
  echo '{"provider":"github","metrics":{"stars":10},"ts":1000}' > "$TMP/before.json"
  echo '{"provider":"github","metrics":{"stars":20},"ts":2000}' > "$TMP/after.json"
}
teardown() { rm -rf "$TMP"; }

@test "empty ledger -> empty table" {
  : > "$TMP/l.jsonl"; run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"
  [ "$status" -eq 0 ]; [ "$(echo "$output" | jq 'length')" = "0" ]
}
@test "single shipping lens gets full positive delta" {
  printf '%s\n' '{"issue":1,"source":"content-opportunity","outcome":"merged_clean","ts":1500}' > "$TMP/l.jsonl"
  run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.[0].source')" = "content-opportunity" ]
  [ "$(echo "$output" | jq -r '.[0].shipped')" = "1" ]
}
@test "two lenses split the delta" {
  printf '%s\n%s\n' \
    '{"issue":1,"source":"content-opportunity","outcome":"merged_clean","ts":1500}' \
    '{"issue":2,"source":"community","outcome":"published","ts":1600}' > "$TMP/l.jsonl"
  run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" = "2" ]
}
@test "malformed ledger fails closed" {
  echo 'not json' > "$TMP/l.jsonl"; run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-attribute.bats`
Expected: FAIL (script not found).

- [ ] **Step 3: Write the attributor**

```bash
#!/usr/bin/env bash
# growth-attribute.sh — deterministic v1 attribution: split the total positive
# metric delta evenly across lenses that shipped in the window. Fail-closed.
set -euo pipefail

before="${1:-}"; after="${2:-}"; ledger="${3:-}"
[ -n "$before" ] && [ -n "$after" ] && [ -n "$ledger" ] || {
  echo "usage: growth-attribute.sh <before.json> <after.json> <ledger.jsonl>" >&2; exit 2; }
for f in "$before" "$after" "$ledger"; do
  [ -f "$f" ] || { echo "not found: $f" >&2; exit 2; }
done
jq . "$before" >/dev/null 2>&1 || { echo "malformed before: $before" >&2; exit 1; }
jq . "$after"  >/dev/null 2>&1 || { echo "malformed after: $after" >&2; exit 1; }
# Ledger is JSONL; validate each line.
if [ -s "$ledger" ] && ! jq -e . "$ledger" >/dev/null 2>&1; then
  echo "malformed ledger (fail-closed): $ledger" >&2; exit 1
fi

# Total positive delta = sum over shared metric keys of max(after-before, 0).
delta="$(jq -n --slurpfile b "$before" --slurpfile a "$after" '
  ($b[0].metrics // {}) as $bm | ($a[0].metrics // {}) as $am
  | [ $am | to_entries[] | select((.value|type)=="number")
      | (.value - (($bm[.key]) // 0)) | select(. > 0) ] | add // 0')"

# Lenses that shipped (merged_clean|published), unique sources.
jq -s --argjson delta "$delta" '
  [ .[] | select(.outcome=="merged_clean" or .outcome=="published") | .source ]
  | unique as $sources
  | ($sources | length) as $n
  | if $n == 0 then []
    else [ $sources[] | { source: ., shipped: 1, delta_share: ($delta / $n) } ]
    end' "$ledger"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/unit/growth-attribute.bats`
Expected: PASS (4/4).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-attribute.sh tests/unit/growth-attribute.bats
git commit -m "feat(grow-run): deterministic v1 attribution (even delta split across shipping lenses)"
```

---

## Task 7: The `/autospec-grow-run` trio skill

**Files:**
- Create: `skills/autospec-grow-run/SKILL.md`
- Create (derived): `skills/autospec-grow-run/codex/prompt.md`, `skills/autospec-grow-run/opencode/agent.md`
- Create: `tests/skill-goldens/autospec-grow-run.sha256` (generated)

**Interfaces:**
- Consumes: all Task 1–6 scripts + Plan 1/2 scripts, `/autospec-run`, `autospec-review/secaudit/persona`, `PushNotification`.
- Produces: the operator-facing `/autospec-grow-run` skill implementing R0–R4 + Stop and the Self-update mode.

Model this skill's structure on `skills/autospec-grow-define/SKILL.md` (same repo): frontmatter, Self-update mode, "Required capabilities & harness adapter" row, "Harness detection + Model tier" section emitting the literal `**Model tier:**` directive, "Relevant memory injection", then the phase sections. Read that file first for the exact section shapes and adapter-row format before writing.

- [ ] **Step 1: Write the failing structural test first**

Add to `autospec validate` a check (fuller wiring is Task 8; here just the named-content asserts so the skill has a target). Create a temporary check invocation to drive authoring:

Run (expected FAIL — file absent):
```bash
test -f skills/autospec-grow-run/SKILL.md && \
  grep -qF '**Model tier:**' skills/autospec-grow-run/SKILL.md && \
  grep -qF '## Self-update mode' skills/autospec-grow-run/SKILL.md && echo OK || echo MISSING
```
Expected: `MISSING`.

- [ ] **Step 2: Write `SKILL.md`**

Author `skills/autospec-grow-run/SKILL.md` with these required sections (prose; no shelled-out user text in Self-update mode). The body must contain, verbatim where noted:

Frontmatter:
```yaml
---
name: autospec-grow-run
description: Drain the growth backlog — implement growth:artifact issues via /autospec-run with a content-quality gate, run the growth:outbound draft→ethics→approval-queue pipeline (package-only, never auto-post), and measure/attribute via live adapters that re-weight the next define cycle. Use after /autospec-grow-define has filed issues.
---
```

Required sections and their load-bearing content:

- `## Self-update mode` — pure prose: when invoked with an update/self-improve request, describe editing `SKILL.md` then re-deriving the trio + regenerating goldens; no bash heredocs of user text.
- `## Required capabilities & harness adapter` — the adapter row table (copy the column shape from `autospec-grow-define`), listing: subagent dispatch, `gh`, `jq`, `curl`, `PushNotification`, browser (optional, best-effort).
- `## Harness detection + Model tier` — resolve `TIER_A`/`TIER_B` once; include the literal line:
  `> **Model tier:** \`TIER_A\` (draft, gate-review, attribute); \`TIER_B\` (cheap mechanical).`
- `## Relevant memory injection` — run-start memory query via `inject-relevant-memory.sh`.
- `## R0 — Detect` — harness detect; load+validate `growth.yml` (`validate-growth-config.sh`); inert exit with a pointer when absent/invalid; resolve the growth-shared script dir once.
- `## R1 — Artifact drain` — enumerate open `growth:artifact` issues via `gh`; per issue invoke `/autospec-run` (invoked, not forked); run the content-quality gate: `growth-content-quality-precheck.sh` deterministic pre-checks THEN a `TIER_A` reviewer, wrapped in a **5-attempt adaptive-retry loop** feeding findings back as directives; the standard reviewer + `growth-ethics` gate + `autospec-secaudit` also run before auto-merge; ledger line → `merged_clean` on success, `failed` on a per-issue failure (drain continues).
- `## R2 — Outbound draft → gate → queue` — per open `growth:outbound`/`growth/needs-draft` issue: draft (`TIER_A`) → `validate-outbound-draft.sh` (malformed → skip+log) → `growth-ethics-precheck.sh --disclosure` + the LLM `growth-ethics` gate (5-attempt retry; blocked → `rejected` ledger line) → `growth-ethics-precheck.sh --cadence` (over cap → refuse) → relevance (draft must carry the venue's self-promo rule from config) → `growth-outbound-queue.sh --build-body` → file `growth/needs-approval` issue in `approval.control_repo` → `pending` ledger line → `PushNotification`.
- `## R3 — Handle approvals` — read each control issue's labels via `growth-outbound-queue.sh --read-state`: `approved` → emit a one-click-publish package as a comment (agent NEVER posts) and record `published`+URL only on human confirmation; `edited` → re-gate + re-queue; `rejected` → `rejected` ledger line.
- `## R4 — Measure & attribute` — run each configured adapter (`growth-adapter-*.sh`); missing creds/fetch error → that provider degrades to `{}` (logged), never aborts; `growth-attribute.sh` over before/after + ledger → per-lens contribution; feed `growth-source-weights.sh` to re-weight and write a learnings memo.
- `## Stop` — print the summary; hand off to the Plan 5 conductor or exit. One pass per invocation.

Reference the "never auto-post" and "fail-closed on missing creds" invariants in the R2/R3/R4 prose.

- [ ] **Step 3: Derive the trio + regenerate goldens**

Run:
```bash
scripts/derive-trio.sh autospec-grow-run --in-place
scripts/gen-skill-goldens.sh autospec-grow-run
```
Expected: `codex/prompt.md` (with a leading blank line) and `opencode/agent.md` created with byte-identical bodies; `tests/skill-goldens/autospec-grow-run.sha256` written.

- [ ] **Step 4: Verify structural asserts + lock-step pass**

Run:
```bash
grep -qF '**Model tier:**' skills/autospec-grow-run/SKILL.md && echo tier-ok
scripts/derive-trio.sh autospec-grow-run --check && echo lockstep-ok
```
Expected: `tier-ok` and `lockstep-ok`.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-grow-run/SKILL.md skills/autospec-grow-run/codex/prompt.md skills/autospec-grow-run/opencode/agent.md tests/skill-goldens/autospec-grow-run.sha256
git commit -m "feat(grow-run): /autospec-grow-run trio skill (R0-R4 orchestration)"
```

---

## Task 8: Wiring — validate.sh, smoke test, installer

**Files:**
- Create: `skills/autospec-grow-run/install.sh`, `skills/autospec-grow-run/uninstall.sh`
- Create: `tests/autospec-grow-run/smoke.bats`
- Modify: `autospec validate`
- Modify: root installer/uninstaller skill lists

**Interfaces:**
- Consumes: Tasks 1–7 outputs.
- Produces: green `autospec validate` and a registered, standalone-installable skill.

- [ ] **Step 1: Write the install.sh (copy autospec-grow-define/install.sh)**

Copy `skills/autospec-grow-define/install.sh` to `skills/autospec-grow-run/install.sh` and adapt: `SKILL_NAME="autospec-grow-run"`, the `AUTOSPEC_GROW_RUN_REF`/`_RAW_BASE` env names, and the `SHARED_LIB_SCRIPT_FILES` list. **Critical (Plan 3 bug):** the new grow-run scripts go in `SHARED_LIB_SCRIPT_FILES` (resolved from `skills/autospec-shared/scripts/`), NOT `SHARED_SCRIPT_FILES`:

```sh
SHARED_LIB_SCRIPT_FILES="inject-relevant-memory.sh validate-growth-config.sh validate-growth-candidate.sh growth-measure.sh growth-source-weights.sh growth-candidate-dedup.sh growth-candidate-verify.sh growth-candidate-rank.sh growth-ledger.sh growth-ethics-blocklist.sh growth-ethics-precheck.sh validate-outbound-draft.sh growth-content-quality-precheck.sh growth-outbound-queue.sh growth-adapter-github.sh growth-adapter-analytics.sh growth-adapter-gsc.sh growth-adapter-rank.sh growth-attribute.sh"
```

Keep the repo-root `SHARED_SCRIPT_FILES` list identical to `autospec-grow-define`'s (the `autospec-usage-limit.sh … gen-reviewer-prompt.sh` set). Copy `uninstall.sh` similarly.

- [ ] **Step 2: bash -n both installers**

Run: `bash -n skills/autospec-grow-run/install.sh && bash -n skills/autospec-grow-run/uninstall.sh && echo ok`
Expected: `ok`.

- [ ] **Step 3: Write the smoke test with subprocess mocks**

```bash
# tests/autospec-grow-run/smoke.bats
# Drives the deterministic spine end-to-end with mock gh / fetch. Subprocess
# mocks (not live subagents) prevent the monitor-stall failure mode.
setup() {
  ROOT="$BATS_TEST_DIRNAME/../.."
  DIR="$ROOT/skills/autospec-shared/scripts"
  TMP="$(mktemp -d)"; export GROWTH_NOW_EPOCH=1000
  export GROWTH_LEDGER="$TMP/ledger.jsonl"; : > "$GROWTH_LEDGER"
}
teardown() { rm -rf "$TMP"; }

@test "valid draft is queued; malformed draft is dropped" {
  cat > "$TMP/good.json" <<'JSON'
{"issue":1,"platform":"reddit","target_url":"https://r/x","body":"Hi.","self_promo_rule":"ok","evidence":[]}
JSON
  run bash "$DIR/validate-outbound-draft.sh" "$TMP/good.json"; [ "$status" -eq 0 ]
  run bash "$DIR/growth-outbound-queue.sh" --build-body "$TMP/good.json"; [ "$status" -eq 0 ]
  [[ "$output" == *"https://r/x"* ]]
  echo '{"body":""}' > "$TMP/bad.json"
  run bash "$DIR/validate-outbound-draft.sh" "$TMP/bad.json"; [ "$status" -ne 0 ]
}

@test "over-cadence draft is refused" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}}}' > "$TMP/cfg.json"
  bash "$DIR/growth-ledger.sh" --append '{"issue":9,"source":"community","platform":"reddit","outcome":"published","ts":1000}' >/dev/null 2>&1 || \
    printf '%s\n' '{"issue":9,"source":"community","platform":"reddit","outcome":"published","ts":1000}' > "$GROWTH_LEDGER"
  run bash "$DIR/growth-ethics-precheck.sh" --cadence "$TMP/cfg.json" "$GROWTH_LEDGER" reddit
  [ "$status" -ne 0 ]
}

@test "approved state produces package path but never posts (no gh in helper)" {
  run bash "$DIR/growth-outbound-queue.sh" --read-state "growth/approved"; [ "$output" = "approved" ]
}

@test "measure degrades to fail-closed on missing creds" {
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"DEFINITELY_UNSET_TOK"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"; [ "$status" -ne 0 ]; [ -z "$output" ]
}
```

- [ ] **Step 4: Run smoke test**

Run: `bats tests/autospec-grow-run/smoke.bats`
Expected: PASS (4/4).

- [ ] **Step 5: Wire validate.sh**

In `autospec validate`, add two functions (model them on `check_grow_define_contract` / `check_growth_candidate_pipeline_contract` already in the file) and register them in `main`'s run list:

```bash
check_grow_run_pipeline_contract() {
  info "check_grow_run_pipeline_contract"
  local d="skills/autospec-shared/scripts"
  local s
  for s in validate-outbound-draft.sh growth-content-quality-precheck.sh \
           growth-outbound-queue.sh growth-attribute.sh \
           growth-adapter-github.sh growth-adapter-analytics.sh \
           growth-adapter-gsc.sh growth-adapter-rank.sh; do
    bash -n "$d/$s" || return 1
  done
  run_bats tests/unit/validate-outbound-draft.bats || return 1
  run_bats tests/unit/growth-content-quality-precheck.bats || return 1
  run_bats tests/unit/growth-outbound-queue.bats || return 1
  run_bats tests/unit/growth-adapters.bats || return 1
  run_bats tests/unit/growth-attribute.bats || return 1
  run_bats tests/unit/growth-measure.bats || return 1
}

check_grow_run_contract() {
  info "check_grow_run_contract"
  local f="skills/autospec-grow-run/SKILL.md"
  [ -f "$f" ] || { err "missing $f"; return 1; }
  grep -qF '**Model tier:**' "$f" || { err "grow-run: missing Model tier directive"; return 1; }
  grep -qF '## Self-update mode' "$f" || { err "grow-run: missing Self-update mode"; return 1; }
  grep -qF '## R1 — Artifact drain' "$f" || { err "grow-run: missing R1"; return 1; }
  grep -qF '## R2 — Outbound' "$f" || { err "grow-run: missing R2"; return 1; }
  grep -qF '## R4 — Measure' "$f" || { err "grow-run: missing R4"; return 1; }
}
```

Match the EXACT helper names already used in `validate.sh` (`info`/`err`/`run_bats` or whatever the file uses — read the neighbours of `check_grow_define_contract` first and mirror them). Register both new checks wherever `check_grow_define_contract` is invoked in `main`.

- [ ] **Step 6: Register the skill in the root installer**

Add `autospec-grow-run` to the root `install.sh` / `uninstall.sh` skill-discovery list exactly as `autospec-grow-define` is listed (grep the root `install.sh` for `autospec-grow-define` and mirror every occurrence).

- [ ] **Step 7: Run the full gate**

Run: `autospec validate`
Expected: `validate: OK — all validation checks passed.`
Also verify the root install still passes with a temp HOME:
```bash
TH="$(mktemp -d)"; HOME="$TH" bash install.sh --hook-mode claude >/dev/null 2>&1 && echo "root-install ok"; rm -rf "$TH"
```
Expected: `root-install ok` and, in the output, `pairs OK (0 failed)`.

- [ ] **Step 8: Commit**

```bash
git add autospec validate skills/autospec-grow-run/install.sh skills/autospec-grow-run/uninstall.sh tests/autospec-grow-run/smoke.bats install.sh uninstall.sh
git commit -m "feat(grow-run): validate wiring + smoke test + installer registration"
```

---

## Self-Review

**Spec coverage:**
- R1 artifact drain + content-quality gate → Task 2 (pre-check) + Task 7 (R1 prose invoking `/autospec-run` + gate + retry). ✓
- R2 outbound draft→ethics→cadence→relevance→queue → Task 1 (draft validator), Task 3 (queue body), reuse ethics precheck, Task 7 (R2 prose). ✓
- R3 approval handling → Task 3 (`--read-state`) + Task 7 (R3 prose, never-post invariant). ✓
- R4 measure & attribute → Task 4 (normalize), Task 5 (adapters), Task 6 (attribute), Task 7 (R4 prose) + reuse `growth-source-weights.sh`. ✓
- Guardrails: cadence + disclosure reused from Plan 1 ethics precheck; content-quality density/citation new (Task 2); fail-closed asserted in every script's tests. ✓
- Install/validate wiring + `SHARED_LIB_SCRIPT_FILES` (Plan 3 bug) → Task 8. ✓

**Placeholder scan:** every code step carries complete, runnable code. Task 7 is prose authoring against a concrete section list with the verbatim `**Model tier:**` line; Task 8 step 5 explicitly says to mirror the existing `check_grow_define_contract` helper names rather than guessing. No TODO/TBD.

**Type consistency:** the outbound-draft record shape `{issue,platform,target_url,body,self_promo_rule,disclosure?,evidence[]}` is used identically in Tasks 1, 3, and 8. The normalized envelope `{provider,metrics,ts}` is consistent across Tasks 4, 5, 6. Ledger fields (`source`, `outcome`, `platform`, `ts`) match Plan 1's `growth-ledger.sh` conventions.

## Notes for the executor (cross-task)

- Tasks 1–6 are independent deterministic scripts; Task 7 depends on 1–6 existing; Task 8 depends on all.
- Reuse, don't duplicate: cadence and FTC disclosure already live in `growth-ethics-precheck.sh` (`--cadence`, `--disclosure`). Do not write new cadence/disclosure scripts.
- Before Task 8's validate wiring, READ the neighbours of `check_grow_define_contract` in `autospec validate` and mirror the exact helper names and `main` registration idiom — the snippets above are illustrative, not verbatim guarantees of the file's local API.
- Any touch to `SKILL.md` after Task 7 requires re-running `derive-trio.sh --in-place` + `gen-skill-goldens.sh` in the same task (validate fails closed otherwise).
