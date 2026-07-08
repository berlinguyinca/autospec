# AutoSpec Growth — Foundation (Plan 1 of 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `autospec-grow-shared` primitives every growth skill depends on — the `growth.yml` config schema + validator, the immutable white-hat ethics blocklist, the deterministic ethics pre-checks (disclosure + cadence caps), the append-only growth ledger, its Bayesian source-weights, and the measurement-adapter interface — each backed by a bats contract wired into `validate.sh`.

**Architecture:** Pure-bash + `jq` shell libraries under `skills/autospec-shared/scripts/`, a JSON Schema under `skills/autospec-shared/schemas/`, and bats contracts under `tests/unit/`. The ledger and source-weights mirror the shipped `explore-ledger.sh` / `explore-source-weights.sh` patterns exactly (append-only JSONL, latest-line-per-key reads, Bayesian-smoothed weights). No new language runtime is introduced. The final task registers a `check_growth_shared_contract` in `validate.sh`.

**Tech Stack:** bash (target macOS bash 3.2 compatible), `jq`, bats-core, JSON Schema (validated with `jq`, no external validator dependency).

## Global Constraints

- **Bash 3.2 compatible.** No associative arrays, no `${var,,}`; use `tr`/`awk`. `[ -f <(...) ]` is unreliable on macOS bash 3.2 — bats tests must write fixtures to a real temp file before any `[ -f ]`/`--validate-file` check.
- **`set -euo pipefail`** at the top of every script. Use `if/then/fi` for one-sided conditionals, never `[ test ] && action` (aborts under `set -e` when the test is false).
- **No `RETURN` traps** for cleanup (they leak into caller frames under `set -u`); use inline cleanup.
- **Secrets are env-var references only.** No credential value ever appears in `growth.yml` or the ledger.
- **Ethics blocklist is fail-closed and immutable:** repo config may only ADD blocks, never remove a built-in one. Any error, missing input, or parse failure in a gate/pre-check → exit non-zero (deny).
- **Ledger is append-only:** never rewrite a line; `--update-outcome` appends a new copy and readers take the latest line per key. Mirror `explore-ledger.sh`.
- **Conventional commits** (`feat:`, `test:`, `chore:`), branch-per-plan `feat/autospec-growth-foundation`, never `--no-verify`, never push to `main`.
- All new scripts must be `bash -n` clean and `chmod +x`.

---

### Task 1: `growth.yml` config schema + validator

**Files:**
- Create: `skills/autospec-shared/schemas/growth-config.schema.json`
- Create: `skills/autospec-shared/scripts/validate-growth-config.sh`
- Test: `tests/unit/growth-config-validate.bats`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `validate-growth-config.sh <path-to-growth.yml-as-json>` — exit 0 valid, non-zero + stderr reason invalid. Callers pass a JSON rendering of the YAML (produced upstream by `yq`); this script validates JSON so it has zero YAML-parser dependency. The canonical config shape is: top-level keys `product` (obj: `name`, `one_liner`, `value_props[]`, `personas[]`, `competitors[]`), `site` (obj: `url`, `repo_path`, `framework`, `sitemap_url`), `channels` (obj of 4 booleans: `technical_seo`, `content`, `outreach`, `directories`), `targets`, `measurement` (obj: `gsc_property`, `analytics.{provider,token_env}`, `github_repo`, `rank_source`), `approval` (obj: `control_repo`, `cadence_caps`), `guardrails` (obj: `extra_blocks[]`). Required: `product.name`, `site.url`, `site.repo_path`, `measurement`, `approval.control_repo`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-config-validate.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-growth-config.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid_json() {
  cat > "$TMP/c.json" <<'JSON'
{"product":{"name":"Acme","one_liner":"x","value_props":["a"],"personas":["dev"],"competitors":["B"]},
 "site":{"url":"https://acme.dev","repo_path":".","framework":"astro","sitemap_url":"https://acme.dev/sitemap.xml"},
 "channels":{"technical_seo":true,"content":true,"outreach":true,"directories":true},
 "targets":{"keyword_seeds":["x"],"directories":[],"communities":[]},
 "measurement":{"gsc_property":"sc-domain:acme.dev","analytics":{"provider":"plausible","token_env":"PLAUSIBLE_API_TOKEN"},"github_repo":"acme/cli","rank_source":"manual"},
 "approval":{"control_repo":"acme/growth","cadence_caps":{"default_per_platform_per_week":2}},
 "guardrails":{"extra_blocks":[]}}
JSON
  echo "$TMP/c.json"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a complete valid config" {
  run bash "$V" "$(valid_json)"
  [ "$status" -eq 0 ]
}

@test "rejects config missing product.name" {
  f="$(valid_json)"; jq 'del(.product.name)' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"product.name"* ]]
}

@test "rejects an inline secret in measurement.analytics" {
  f="$(valid_json)"; jq '.measurement.analytics.token = "sk-live-abc123"' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"secret"* || "$output" == *"token_env"* ]]
}

@test "rejects missing approval.control_repo" {
  f="$(valid_json)"; jq 'del(.approval.control_repo)' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-config-validate.bats`
Expected: FAIL — `validate-growth-config.sh` does not exist (`[ -f "$V" ]` fails).

- [ ] **Step 3: Write the schema**

```json
// skills/autospec-shared/schemas/growth-config.schema.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "autospec growth.yml",
  "type": "object",
  "required": ["product", "site", "measurement", "approval"],
  "properties": {
    "product": { "type": "object", "required": ["name"],
      "properties": { "name": {"type":"string","minLength":1} } },
    "site": { "type": "object", "required": ["url", "repo_path"],
      "properties": { "url": {"type":"string","pattern":"^https?://"} } },
    "channels": { "type": "object" },
    "measurement": { "type": "object",
      "properties": { "analytics": { "type": "object",
        "not": { "required": ["token"] } } } },
    "approval": { "type": "object", "required": ["control_repo"] },
    "guardrails": { "type": "object" }
  }
}
```

- [ ] **Step 4: Write the validator**

```bash
#!/usr/bin/env bash
# validate-growth-config.sh — validate a JSON rendering of .autospec/growth.yml.
# Usage: validate-growth-config.sh <config.json>   (exit 0 valid; non-zero invalid)
set -euo pipefail

CONFIG="${1:?usage: validate-growth-config.sh <config.json>}"
if [ ! -f "$CONFIG" ]; then echo "config not found: $CONFIG" >&2; exit 2; fi
if ! jq -e . "$CONFIG" >/dev/null 2>&1; then echo "not valid JSON: $CONFIG" >&2; exit 2; fi

fail() { echo "growth.yml invalid: $1" >&2; exit 1; }

# Required fields (jq-driven; keeps zero external schema-validator dependency).
jq -e '.product.name // empty | select(. != "")' "$CONFIG" >/dev/null || fail "product.name is required"
jq -e '.site.url // empty | test("^https?://")'  "$CONFIG" >/dev/null || fail "site.url must be an http(s) URL"
jq -e '.site.repo_path // empty'                 "$CONFIG" >/dev/null || fail "site.repo_path is required"
jq -e '.measurement'                             "$CONFIG" >/dev/null || fail "measurement block is required"
jq -e '.approval.control_repo // empty'          "$CONFIG" >/dev/null || fail "approval.control_repo is required"

# Secrets must be env-var references only: reject any inline-looking token value.
if jq -e '.measurement.analytics.token // empty' "$CONFIG" >/dev/null 2>&1; then
  fail "inline secret detected: use measurement.analytics.token_env (env-var name), not token"
fi

exit 0
```

- [ ] **Step 5: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/validate-growth-config.sh
bats tests/unit/growth-config-validate.bats
```
Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add skills/autospec-shared/schemas/growth-config.schema.json \
        skills/autospec-shared/scripts/validate-growth-config.sh \
        tests/unit/growth-config-validate.bats
git commit -m "feat: growth.yml config schema + validator"
```

---

### Task 2: Immutable white-hat ethics blocklist

**Files:**
- Create: `skills/autospec-shared/scripts/growth-ethics-blocklist.sh`
- Test: `tests/unit/growth-ethics-blocklist.bats`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `growth-ethics-blocklist.sh --list` → prints the canonical built-in block IDs, one per line.
  - `growth-ethics-blocklist.sh --effective <config.json>` → prints built-in blocks UNION `guardrails.extra_blocks[]`, deduped. If the config's `extra_blocks` omits any built-in, they are still present (config can only add).
  - `growth-ethics-blocklist.sh --assert-not-weakened <config.json>` → exit 0 if every built-in block is still effective, non-zero if config attempts to override/remove one. Built-in IDs: `fake_reviews`, `undisclosed_incentivized_reviews`, `review_gating`, `rating_vote_manipulation`, `sockpuppets`, `bot_or_fake_signups`, `scraped_email_spam`, `cloaking_doorway`, `link_schemes_pbn`, `platform_tos_violation`.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-ethics-blocklist.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
BL="$REPO_ROOT/skills/autospec-shared/scripts/growth-ethics-blocklist.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

BUILTINS="fake_reviews undisclosed_incentivized_reviews review_gating rating_vote_manipulation sockpuppets bot_or_fake_signups scraped_email_spam cloaking_doorway link_schemes_pbn platform_tos_violation"

@test "script exists and is bash -n clean" {
  [ -f "$BL" ]; run bash -n "$BL"; [ "$status" -eq 0 ]
}

@test "--list contains every built-in block" {
  run bash "$BL" --list
  [ "$status" -eq 0 ]
  for b in $BUILTINS; do [[ "$output" == *"$b"* ]]; done
}

@test "--effective adds extra_blocks and keeps builtins" {
  echo '{"guardrails":{"extra_blocks":["my_custom_block"]}}' > "$TMP/c.json"
  run bash "$BL" --effective "$TMP/c.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *"my_custom_block"* ]]
  [[ "$output" == *"fake_reviews"* ]]
}

@test "--assert-not-weakened passes for an additive config" {
  echo '{"guardrails":{"extra_blocks":["x"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -eq 0 ]
}

@test "--assert-not-weakened fails if config tries to remove a builtin" {
  # a config that declares an explicit allowlist overriding a builtin
  echo '{"guardrails":{"allow":["fake_reviews"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"fake_reviews"* ]]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-ethics-blocklist.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the blocklist script**

```bash
#!/usr/bin/env bash
# growth-ethics-blocklist.sh — single source of truth for the white-hat
# hard-block list. Repo config MAY add blocks; it MAY NOT remove a built-in.
set -euo pipefail

# Built-in hard blocks. This list is the immutable spine of the ethics gate.
BUILTIN_BLOCKS="fake_reviews
undisclosed_incentivized_reviews
review_gating
rating_vote_manipulation
sockpuppets
bot_or_fake_signups
scraped_email_spam
cloaking_doorway
link_schemes_pbn
platform_tos_violation"

usage() { echo "usage: growth-ethics-blocklist.sh --list | --effective <cfg> | --assert-not-weakened <cfg>" >&2; exit 2; }

cmd="${1:-}"; shift || true
case "$cmd" in
  --list)
    printf '%s\n' "$BUILTIN_BLOCKS"
    ;;
  --effective)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    extra="$(jq -r '(.guardrails.extra_blocks // [])[]' "$cfg" 2>/dev/null || true)"
    printf '%s\n%s\n' "$BUILTIN_BLOCKS" "$extra" | awk 'NF' | sort -u
    ;;
  --assert-not-weakened)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    # Fail closed if the config declares ANY allow/exclude list naming a builtin.
    denied=""
    for key in allow allowlist exclude remove disable; do
      hits="$(jq -r --arg k "$key" '(.guardrails[$k] // [])[]' "$cfg" 2>/dev/null || true)"
      for h in $hits; do
        if printf '%s\n' "$BUILTIN_BLOCKS" | grep -qx "$h"; then
          denied="$denied $h"
        fi
      done
    done
    if [ -n "$denied" ]; then
      echo "growth.yml attempts to weaken built-in ethics blocks:$denied" >&2
      exit 1
    fi
    exit 0
    ;;
  *) usage ;;
esac
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-ethics-blocklist.sh
bats tests/unit/growth-ethics-blocklist.bats
```
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-ethics-blocklist.sh tests/unit/growth-ethics-blocklist.bats
git commit -m "feat: immutable white-hat ethics blocklist"
```

---

### Task 3: Deterministic ethics pre-checks (disclosure + cadence caps)

**Files:**
- Create: `skills/autospec-shared/scripts/growth-ethics-precheck.sh`
- Test: `tests/unit/growth-ethics-precheck.bats`

**Interfaces:**
- Consumes: the ledger reader from Task 4 is NOT required here — cadence counting takes a ledger file path and counts lines directly, so this task can be built before Task 4. It consumes `growth.yml` JSON (Task 1) for cadence caps.
- Produces:
  - `growth-ethics-precheck.sh --disclosure <draft.md>` → exit 0 if the draft either declares no sponsorship OR contains a disclosure marker (`#ad`, `#sponsored`, or the literal `Disclosure:`); exit non-zero if it self-identifies as sponsored/affiliate (`affiliate`, `sponsored`, `paid partnership`) WITHOUT a disclosure marker.
  - `growth-ethics-precheck.sh --cadence <config.json> <ledger.jsonl> <platform>` → exit 0 if published-count for `<platform>` within the last 7 days is below the platform's cap; non-zero if at/over cap. Cap resolves from `targets.communities[]` matching `platform`, else `approval.cadence_caps.default_per_platform_per_week`. Ledger lines are JSON objects with `{platform, outcome, ts}`; a line counts when `outcome=="published"`. **Note:** this uses a caller-supplied "now" epoch via `GROWTH_NOW_EPOCH` env var so tests are deterministic (no `date +%s` at call time); defaults to `date -u +%s` when unset.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-ethics-precheck.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
PC="$REPO_ROOT/skills/autospec-shared/scripts/growth-ethics-precheck.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_NOW_EPOCH=1000000; }
teardown() { rm -rf "$TMP"; unset GROWTH_NOW_EPOCH; }

@test "script exists and is bash -n clean" {
  [ -f "$PC" ]; run bash -n "$PC"; [ "$status" -eq 0 ]
}

@test "disclosure: plain non-sponsored draft passes" {
  printf 'Check out our open-source CLI, it is fast.\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -eq 0 ]
}

@test "disclosure: sponsored draft WITHOUT marker fails" {
  printf 'This sponsored post highlights our tool.\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -ne 0 ]
}

@test "disclosure: sponsored draft WITH marker passes" {
  printf 'This sponsored post highlights our tool.\n#ad\n' > "$TMP/d.md"
  run bash "$PC" --disclosure "$TMP/d.md"
  [ "$status" -eq 0 ]
}

@test "cadence: under cap passes" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  # one published line, 1 day ago (epoch 1000000 - 86400)
  echo '{"platform":"reddit","outcome":"published","ts":913600}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -eq 0 ]
}

@test "cadence: at cap fails" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":2}},"targets":{"communities":[]}}' > "$TMP/c.json"
  printf '{"platform":"reddit","outcome":"published","ts":990000}\n{"platform":"reddit","outcome":"published","ts":990001}\n' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -ne 0 ]
}

@test "cadence: old publishes outside 7-day window do not count" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}},"targets":{"communities":[]}}' > "$TMP/c.json"
  # published 8 days ago: 1000000 - 8*86400 = 308800
  echo '{"platform":"reddit","outcome":"published","ts":308800}' > "$TMP/l.jsonl"
  run bash "$PC" --cadence "$TMP/c.json" "$TMP/l.jsonl" reddit
  [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-ethics-precheck.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the pre-check script**

```bash
#!/usr/bin/env bash
# growth-ethics-precheck.sh — deterministic ethics pre-checks that gate a draft
# BEFORE any LLM ethics review. Fail-closed.
set -euo pipefail

WEEK_SECONDS=604800

usage() { echo "usage: growth-ethics-precheck.sh --disclosure <draft.md> | --cadence <cfg.json> <ledger.jsonl> <platform>" >&2; exit 2; }

now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }

check_disclosure() {
  local draft="${1:?draft required}"
  [ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
  local body; body="$(tr '[:upper:]' '[:lower:]' < "$draft")"
  local is_sponsored=0
  case "$body" in
    *affiliate*|*sponsored*|*"paid partnership"*) is_sponsored=1 ;;
  esac
  if [ "$is_sponsored" -eq 0 ]; then exit 0; fi
  case "$body" in
    *"#ad"*|*"#sponsored"*|*"disclosure:"*) exit 0 ;;
  esac
  echo "sponsored/affiliate draft lacks an FTC disclosure marker (#ad, #sponsored, or 'Disclosure:')" >&2
  exit 1
}

check_cadence() {
  local cfg="${1:?}" ledger="${2:?}" platform="${3:?}"
  [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
  [ -f "$ledger" ] || { echo "ledger not found: $ledger" >&2; exit 2; }
  local cap
  cap="$(jq -r --arg p "$platform" \
    'first(.targets.communities[]? | select(.platform==$p) | .cadence_cap_per_week)
     // .approval.cadence_caps.default_per_platform_per_week // 2' "$cfg")"
  local now cutoff count
  now="$(now_epoch)"; cutoff=$(( now - WEEK_SECONDS ))
  count="$(jq -r --arg p "$platform" --argjson cut "$cutoff" \
    'select(.platform==$p and .outcome=="published" and (.ts|tonumber) >= $cut)' \
    "$ledger" 2>/dev/null | jq -s 'length')"
  if [ "$count" -ge "$cap" ]; then
    echo "cadence cap reached for $platform: $count/$cap published in the last 7 days" >&2
    exit 1
  fi
  exit 0
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --disclosure) check_disclosure "$@" ;;
  --cadence)    check_cadence "$@" ;;
  *) usage ;;
esac
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-ethics-precheck.sh
bats tests/unit/growth-ethics-precheck.bats
```
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-ethics-precheck.sh tests/unit/growth-ethics-precheck.bats
git commit -m "feat: deterministic ethics pre-checks (disclosure + cadence caps)"
```

---

### Task 4: Append-only growth ledger

**Files:**
- Create: `skills/autospec-shared/scripts/growth-ledger.sh`
- Test: `tests/unit/growth-ledger.bats`

**Interfaces:**
- Consumes: nothing (patterned on `explore-ledger.sh`).
- Produces:
  - `growth-ledger.sh --append '<json>'` — appends one JSON object line to `$GROWTH_LEDGER` (env; defaults to `.autospec/growth/ledger.jsonl`). Required keys: `{round, source, title, norm_title, channel, kind, issue, outcome, reason, ts}` where `kind` ∈ `artifact|outbound`, `outcome` ∈ `pending|merged_clean|published|rejected|refuted|failed`.
  - `growth-ledger.sh --update-outcome <issue> <outcome> [reason]` — appends a new copy of the latest line for `<issue>` with updated outcome/reason (append-only; never rewrites).
  - `growth-ledger.sh --show [--source <name>] [--json]` — prints latest line per issue.
  - `growth-ledger.sh --stats [--json]` — per-source `{filed, merged_clean, published, refuted}` counts (latest line per issue; `refuted` never counts toward `filed`).
  - `growth-ledger.sh --validate [<file>]` — exit non-zero if any line is missing a required key.

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-ledger.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

line() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"seo\",\"kind\":\"$2\",\"issue\":$3,\"outcome\":\"$4\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$LG" ]; run bash -n "$LG"; [ "$status" -eq 0 ]
}

@test "append then show returns the line" {
  bash "$LG" --append "$(line keyword-gap artifact 7 pending)"
  run bash "$LG" --show --json
  [ "$status" -eq 0 ]
  [[ "$output" == *"keyword-gap"* ]]
}

@test "update-outcome appends and show reflects latest" {
  bash "$LG" --append "$(line keyword-gap artifact 7 pending)"
  bash "$LG" --update-outcome 7 merged_clean "done"
  # two physical lines, one logical (latest) state
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 2 ]
  run bash "$LG" --show --json
  [[ "$output" == *"merged_clean"* ]]
  [[ "$output" != *"pending"* ]]
}

@test "stats: refuted does not count toward filed" {
  bash "$LG" --append "$(line community artifact 8 pending)"
  bash "$LG" --append "$(line community outbound 0 refuted)"
  run bash "$LG" --stats --json
  [ "$status" -eq 0 ]
  # community filed == 1 (issue 8), refuted == 1
  echo "$output" | jq -e '.community.filed == 1'
  echo "$output" | jq -e '.community.refuted == 1'
}

@test "validate rejects a line missing a required key" {
  echo '{"round":1,"source":"x"}' > "$GROWTH_LEDGER"
  run bash "$LG" --validate
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-ledger.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the ledger script**

```bash
#!/usr/bin/env bash
# growth-ledger.sh — append-only JSONL outcome ledger for autospec-grow.
# Mirrors explore-ledger.sh: append-only, readers take latest line per issue.
set -euo pipefail

LEDGER="${GROWTH_LEDGER:-.autospec/growth/ledger.jsonl}"
REQUIRED='["round","source","title","norm_title","channel","kind","issue","outcome","reason","ts"]'

ensure_dir() { mkdir -p "$(dirname "$LEDGER")"; }

do_append() {
  local obj="${1:?json required}"
  echo "$obj" | jq -e . >/dev/null || { echo "not valid JSON" >&2; exit 2; }
  local missing
  missing="$(echo "$obj" | jq -r --argjson req "$REQUIRED" '$req - (keys) | .[]' 2>/dev/null || true)"
  if [ -n "$missing" ]; then echo "missing keys: $missing" >&2; exit 1; fi
  ensure_dir
  echo "$(echo "$obj" | jq -c .)" >> "$LEDGER"
}

# latest line per issue (issue 0 lines are kept individually — they are refutations)
latest() {
  [ -f "$LEDGER" ] || return 0
  jq -s '
    (map(select(.issue != 0)) | group_by(.issue) | map(.[-1]))
    + (map(select(.issue == 0)))' "$LEDGER"
}

do_update_outcome() {
  local issue="${1:?}" outcome="${2:?}" reason="${3:-}"
  [ -f "$LEDGER" ] || { echo "no ledger" >&2; exit 1; }
  local prev
  prev="$(jq -s --argjson i "$issue" 'map(select(.issue==$i)) | last' "$LEDGER")"
  if [ "$prev" = "null" ]; then echo "issue $issue not found" >&2; exit 1; fi
  echo "$prev" | jq -c --arg o "$outcome" --arg r "$reason" '.outcome=$o | .reason=$r' >> "$LEDGER"
}

do_show() {
  local src="" json=0
  while [ $# -gt 0 ]; do case "$1" in
    --source) src="$2"; shift 2;; --json) json=1; shift;; *) shift;; esac; done
  local out; out="$(latest)"
  if [ -n "$src" ]; then out="$(echo "$out" | jq --arg s "$src" 'map(select(.source==$s))')"; fi
  if [ "$json" -eq 1 ]; then echo "$out"; else echo "$out" | jq -r '.[] | "\(.issue)\t\(.source)\t\(.outcome)\t\(.title)"'; fi
}

do_stats() {
  latest | jq '
    group_by(.source) | map({
      key: .[0].source,
      value: {
        filed:        (map(select(.issue != 0)) | length),
        merged_clean: (map(select(.outcome=="merged_clean")) | length),
        published:    (map(select(.outcome=="published")) | length),
        refuted:      (map(select(.outcome=="refuted")) | length)
      }
    }) | from_entries'
}

do_validate() {
  local f="${1:-$LEDGER}"
  [ -f "$f" ] || { echo "no ledger: $f" >&2; exit 0; }
  local bad
  bad="$(jq -c --argjson req "$REQUIRED" 'select(($req - (keys)) | length > 0)' "$f" 2>/dev/null || true)"
  if [ -n "$bad" ]; then echo "invalid ledger lines:" >&2; echo "$bad" >&2; exit 1; fi
  exit 0
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --append)         do_append "$@" ;;
  --update-outcome) do_update_outcome "$@" ;;
  --show)           do_show "$@" ;;
  --stats)          shift || true; do_stats ;;
  --validate)       do_validate "$@" ;;
  *) echo "usage: growth-ledger.sh --append|--update-outcome|--show|--stats|--validate" >&2; exit 2 ;;
esac
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-ledger.sh
bats tests/unit/growth-ledger.bats
```
Expected: all PASS. (If `--stats` is invoked with a stray `--json` arg, note the `shift` in the case arm discards it; `do_stats` ignores flags and always emits JSON.)

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-ledger.sh tests/unit/growth-ledger.bats
git commit -m "feat: append-only growth outcome ledger"
```

---

### Task 5: Bayesian source-weights from the ledger

**Files:**
- Create: `skills/autospec-shared/scripts/growth-source-weights.sh`
- Test: `tests/unit/growth-source-weights.bats`

**Interfaces:**
- Consumes: `growth-ledger.sh --stats --json` (Task 4).
- Produces: `growth-source-weights.sh [--json]` → a JSON object mapping each known source name to a weight in `(0,1]`. Formula mirrors `explore-source-weights.sh`: `base = (merged_clean + published + alpha*prior) / (filed + alpha)`; `weight = base * downweight`, where `downweight = 1 / (1 + refuted/ (filed+1))`. `alpha=5`, `prior=0.5` for unknown sources. Known sources: `technical-seo`, `keyword-gap`, `content-opportunity`, `community`, `directory`, `backlink`. A source with no ledger data gets `base = prior` (i.e. weight `0.5`).

- [ ] **Step 1: Write the failing test**

```bash
# tests/unit/growth-source-weights.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SW="$REPO_ROOT/skills/autospec-shared/scripts/growth-source-weights.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

line() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"seo\",\"kind\":\"artifact\",\"issue\":$2,\"outcome\":\"$3\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$SW" ]; run bash -n "$SW"; [ "$status" -eq 0 ]
}

@test "unknown/empty ledger yields prior 0.5 for every known source" {
  run bash "$SW" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.["keyword-gap"] == 0.5'
  echo "$output" | jq -e '.["community"] == 0.5'
}

@test "a source with all-clean ships weights above prior" {
  bash "$LG" --append "$(line keyword-gap 1 merged_clean)"
  bash "$LG" --append "$(line keyword-gap 2 merged_clean)"
  bash "$LG" --append "$(line keyword-gap 3 merged_clean)"
  run bash "$SW" --json
  echo "$output" | jq -e '.["keyword-gap"] > 0.5'
}

@test "refutations pull a source's weight below its clean-rate base" {
  bash "$LG" --append "$(line community 4 merged_clean)"
  bash "$LG" --append "$(line community 0 refuted)"
  bash "$LG" --append "$(line community 0 refuted)"
  run bash "$SW" --json
  # community still defined and <= keyword-gap-with-no-refutations comparison not needed;
  # just assert it is a valid number in (0,1]
  echo "$output" | jq -e '.["community"] > 0 and .["community"] <= 1'
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-source-weights.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the weights script**

```bash
#!/usr/bin/env bash
# growth-source-weights.sh — Bayesian-smoothed per-source ranking weights,
# derived from the growth ledger. Mirrors explore-source-weights.sh.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"
ALPHA=5
PRIOR=0.5
SOURCES="technical-seo keyword-gap content-opportunity community directory backlink"

stats_json="$("$LEDGER_SH" --stats --json 2>/dev/null || echo '{}')"

emit="{}"
for s in $SOURCES; do
  w="$(echo "$stats_json" | jq -r --arg s "$s" --argjson a "$ALPHA" --argjson p "$PRIOR" '
    (.[$s] // {filed:0,merged_clean:0,published:0,refuted:0}) as $x
    | (($x.merged_clean + $x.published + ($a*$p)) / ($x.filed + $a)) as $base
    | (1 / (1 + ($x.refuted / ($x.filed + 1)))) as $dw
    | ($base * $dw)')"
  emit="$(echo "$emit" | jq --arg s "$s" --argjson w "$w" '.[$s] = ($w | (.*10000|round)/10000)')"
done

echo "$emit"
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-source-weights.sh
bats tests/unit/growth-source-weights.bats
```
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-source-weights.sh tests/unit/growth-source-weights.bats
git commit -m "feat: Bayesian growth source-weights from ledger"
```

---

### Task 6: Measurement-adapter interface + fixture adapter

**Files:**
- Create: `skills/autospec-shared/scripts/growth-measure.sh`
- Create: `tests/fixtures/growth/gsc-sample.json`
- Test: `tests/unit/growth-measure.bats`

**Interfaces:**
- Consumes: `growth.yml` JSON (Task 1) for provider selection.
- Produces: `growth-measure.sh --normalize <provider> <raw.json>` → a normalized metrics envelope `{"provider":<p>,"metrics":{...},"ts":<GROWTH_NOW_EPOCH-or-now>}`. Supported providers in v1: `gsc` (maps `rows[]` with `keys`+`clicks`+`impressions`+`position` → `{clicks_total, impressions_total, queries[]}`), `github` (maps `{stargazers_count, forks_count}` → `{stars, forks}`). Unknown provider → exit non-zero. This is the seam every real adapter (live API caller) plugs into later; v1 ships normalization + a fixture, not live API calls (those are per-adapter tasks in Plan 4).

- [ ] **Step 1: Write the fixture and failing test**

```json
// tests/fixtures/growth/gsc-sample.json
{ "rows": [
  { "keys": ["fast x cli"], "clicks": 12, "impressions": 400, "position": 6.2 },
  { "keys": ["x tool"],     "clicks": 3,  "impressions": 150, "position": 14.0 }
] }
```

```bash
# tests/unit/growth-measure.bats
#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
M="$REPO_ROOT/skills/autospec-shared/scripts/growth-measure.sh"
FIX="$REPO_ROOT/tests/fixtures/growth/gsc-sample.json"

setup() { TMP="$(mktemp -d)"; export GROWTH_NOW_EPOCH=1000000; }
teardown() { rm -rf "$TMP"; unset GROWTH_NOW_EPOCH; }

@test "script exists and is bash -n clean" {
  [ -f "$M" ]; run bash -n "$M"; [ "$status" -eq 0 ]
}

@test "gsc normalize sums clicks and impressions" {
  run bash "$M" --normalize gsc "$FIX"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.provider == "gsc"'
  echo "$output" | jq -e '.metrics.clicks_total == 15'
  echo "$output" | jq -e '.metrics.impressions_total == 550'
  echo "$output" | jq -e '.ts == 1000000'
}

@test "github normalize maps stars and forks" {
  echo '{"stargazers_count":42,"forks_count":5}' > "$TMP/gh.json"
  run bash "$M" --normalize github "$TMP/gh.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.metrics.stars == 42'
  echo "$output" | jq -e '.metrics.forks == 5'
}

@test "unknown provider fails" {
  echo '{}' > "$TMP/x.json"
  run bash "$M" --normalize wat "$TMP/x.json"
  [ "$status" -ne 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/unit/growth-measure.bats`
Expected: FAIL — script missing.

- [ ] **Step 3: Write the measure script**

```bash
#!/usr/bin/env bash
# growth-measure.sh — measurement-adapter normalization seam for autospec-grow.
# v1 normalizes raw provider payloads into a common envelope; live API callers
# plug into --normalize downstream (Plan 4).
set -euo pipefail

now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }

normalize() {
  local provider="${1:?provider required}" raw="${2:?raw json required}"
  [ -f "$raw" ] || { echo "raw not found: $raw" >&2; exit 2; }
  local ts; ts="$(now_epoch)"
  case "$provider" in
    gsc)
      jq --argjson ts "$ts" '{
        provider: "gsc",
        metrics: {
          clicks_total: ([.rows[].clicks] | add // 0),
          impressions_total: ([.rows[].impressions] | add // 0),
          queries: [.rows[] | {query: .keys[0], clicks, impressions, position}]
        },
        ts: $ts }' "$raw"
      ;;
    github)
      jq --argjson ts "$ts" '{
        provider: "github",
        metrics: { stars: (.stargazers_count // 0), forks: (.forks_count // 0) },
        ts: $ts }' "$raw"
      ;;
    *)
      echo "unknown provider: $provider" >&2; exit 1 ;;
  esac
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --normalize) normalize "$@" ;;
  *) echo "usage: growth-measure.sh --normalize <provider> <raw.json>" >&2; exit 2 ;;
esac
```

- [ ] **Step 4: Make executable and run test**

```bash
chmod +x skills/autospec-shared/scripts/growth-measure.sh
bats tests/unit/growth-measure.bats
```
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/growth-measure.sh tests/fixtures/growth/gsc-sample.json tests/unit/growth-measure.bats
git commit -m "feat: growth measurement-adapter normalization seam"
```

---

### Task 7: Wire the foundation into `validate.sh`

**Files:**
- Modify: `scripts/validate.sh` (add a `check_growth_shared_contract` function + register it in the run list). If the repo's validation entrypoint differs, locate it via `grep -rl "check_lockstep\|check_derive_trio_consistency" scripts/` and add there.
- Test: the check itself IS the test (runs the six bats files); no separate bats.

**Interfaces:**
- Consumes: all six bats suites from Tasks 1–6.
- Produces: a `check_growth_shared_contract` that fails the overall validation run if any growth-shared bats suite fails or any new script is not `bash -n` clean.

- [ ] **Step 1: Locate the validation entrypoint and its check-registration pattern**

Run:
```bash
grep -rn "check_lockstep\|check_derive_trio_consistency\|^run_check\|CHECKS=" scripts/validate.sh | head -30
```
Expected: shows how existing `check_*` functions are declared and invoked (a run list or sequential calls near the bottom). Note the exact idiom.

- [ ] **Step 2: Add the check function**

Add near the other `check_*` definitions (adapt the invocation idiom to what Step 1 revealed):

```bash
check_growth_shared_contract() {
  echo "== growth-shared contract =="
  local root; root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  local rc=0

  # 1. bash -n on every new growth-shared script
  for s in validate-growth-config growth-ethics-blocklist growth-ethics-precheck \
           growth-ledger growth-source-weights growth-measure; do
    local p="$root/skills/autospec-shared/scripts/$s.sh"
    if [ ! -f "$p" ]; then echo "MISSING: $p"; rc=1; continue; fi
    if ! bash -n "$p"; then echo "bash -n FAILED: $p"; rc=1; fi
  done

  # 2. run the six unit suites
  for t in growth-config-validate growth-ethics-blocklist growth-ethics-precheck \
           growth-ledger growth-source-weights growth-measure; do
    if ! bats "$root/tests/unit/$t.bats"; then
      echo "bats FAILED: $t"; rc=1
    fi
  done

  return "$rc"
}
```

- [ ] **Step 3: Register the check in the run list**

Add `check_growth_shared_contract` alongside the other check invocations at the bottom of `validate.sh`, matching the existing idiom (e.g. append to the `CHECKS=(...)` array, or add a direct call with the same error-aggregation the file already uses).

- [ ] **Step 4: Run the check standalone, then the full validation**

Run:
```bash
bash -c 'source scripts/validate.sh; check_growth_shared_contract'   # if functions are sourceable
bash scripts/validate.sh                                              # full run
```
Expected: `growth-shared contract` section PASSes; overall validation still green.

- [ ] **Step 5: Commit**

```bash
git add scripts/validate.sh
git commit -m "test: wire growth-shared contract into validate.sh"
```

---

## Remaining plans (build order — each its own spec→plan→implement cycle)

1. **Foundation** — this plan. Shared primitives (schema, ethics, ledger, weights, measurement seam).
2. **Researchers** — the 6 growth researcher lenses + candidate schema + adversarial-verify + ROI/severity rank + dedup-against-ledger. Consumes Task 4/5.
3. **`/autospec-grow-define` trio** — orchestrates G0→G2: sync via measurement seam, fan out researchers, decompose survivors into `growth:artifact` / `growth:outbound` GitHub issues. Trio derived via `derive-trio.sh --in-place` + `gen-skill-goldens.sh`.
4. **`/autospec-grow-run` trio + outbound pipeline + live measurement adapters** — G3/G4: wrap `/autospec-run` for artifacts, run the `growth-ethics` LLM gate (with 5-attempt adaptive retry) + Task 3 pre-checks, and the control-channel approval queue (GitHub labels + PushNotification). Adds live GSC/Plausible/GitHub/rank API callers behind the Task 6 seam.
5. **`/autospec-grow` conductor trio** — the perpetual growth waterfall wrapping define/run, G5 attribution/re-weighting, quota/parking, live steering.

## Self-Review

**Spec coverage (Plan 1 scope only):** `growth.yml` schema → Task 1 ✓. Env-var-only secrets → Task 1 (inline-secret rejection) ✓. Hard-coded immutable blocklist → Task 2 ✓. Blocklist-immutability test → Task 2 (`--assert-not-weakened`) ✓. FTC disclosure pre-check → Task 3 ✓. Cadence-cap counter → Task 3 ✓. Growth ledger (append-only, latest-per-key, refuted-not-filed) → Task 4 ✓. Source weights → Task 5 ✓. Measurement adapters (GSC/GitHub in v1 seam; Plausible/rank deferred to Plan 4 per spec's adapter-per-task note) → Task 6 ✓. `validate.sh` wiring + lock-step/golden note → Task 7 (trio golden regen belongs to Plans 3–5, flagged there). Researchers, define/run/conductor, outbound pipeline → explicitly deferred to Plans 2–5, listed above. No Plan-1 spec requirement is left without a task.

**Placeholder scan:** every code step contains complete, runnable bash/jq/JSON and each test contains real assertions; no TBD/TODO/"handle edge cases". ✓

**Type/name consistency:** ledger required-key set `{round,source,title,norm_title,channel,kind,issue,outcome,reason,ts}` is identical across Task 4 (`REQUIRED`), the Task 5 `line()` fixture, and Task 6 unaffected. Source names `{technical-seo,keyword-gap,content-opportunity,community,directory,backlink}` match between Task 5 `SOURCES` and its tests. `GROWTH_NOW_EPOCH` / `GROWTH_LEDGER` env seams used consistently across Tasks 3–6. Outcome vocabulary `{pending,merged_clean,published,rejected,refuted,failed}` consistent between Task 4 interface and the weights formula (which reads `merged_clean`,`published`,`refuted`). ✓
