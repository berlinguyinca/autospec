#!/usr/bin/env bats
# tests/unit/filing-origin-self-explore-growth.bats — issue #1745.
#
# Closes the spec's grep-audit (docs/specs/2026-07-10-autonomous-integration-branch-design.md
# §Architecture item 4): the last two remaining autonomous `gh issue create`
# fallback sites — scripts/autospec-explore.sh's raw per-round filing
# (`_explore_raw_file_round`, both the primary and stderr-visible retry call)
# and skills/autospec-shared/scripts/grow-define-file-issues.sh's growth
# filing call — must stamp `origin:self` at creation time, mirroring the
# `origin:self` provenance pattern already merged for Tier 2/3 self-improvement
# and resilience filing sites (PR #1756). Label auto-creation is idempotent
# and best-effort: a `gh label create` failure never aborts filing.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
EXPLORE="$REPO_ROOT/scripts/autospec-explore.sh"
GROW_DEFINE="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh"
LEDGER_SH="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() {
    TMP="$(mktemp -d -t filing-origin-self.XXXXXX)"
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR"
    export PATH="$STUB_DIR:$PATH"
    export GH_LOG="$TMP/gh.log"
}

teardown() {
    rm -rf "$TMP"
}

# ── scripts/autospec-explore.sh: _explore_raw_file_round fallback filing ────

_explore_gh_stub() {
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit "${GH_LABEL_EXIT:-0}" ;;
  *"issue create"*) printf 'https://github.com/berlinguyinca/autospec/issues/321\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$STUB_DIR/gh"
}

_run_explore_raw_filing() {
    mkdir -p "$TMP/iter"
    cat > "$TMP/proposals.json" <<'JSON'
{
  "proposals": [
    {"title": "feat: add retry", "source": "spec-vs-code", "estimated_complexity": "small", "confidence": 0.9}
  ]
}
JSON
    cat > "$TMP/driver.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
SANDBOX_BRANCH="sandbox/explore-demo"
RESEARCH_SOURCES="spec-vs-code"
iter=1
research_json="$TMP/proposals.json"
iter_dir="$TMP/iter"
proposals_count=1
issues_filed=0
filed_issue_nums=""
_ledger_append() { :; }
_ledger_normalize_title() { printf '%s' "\$1"; }
LEDGER_BIN=""
$(awk '/^# >>> explore-spec-first-filing >>>/,/^# <<< explore-spec-first-filing <<</' "$EXPLORE")
_explore_raw_file_round
DRIVER
    bash "$TMP/driver.sh"
}

@test "explore fallback filing: primary gh issue create carries --label origin:self" {
    _explore_gh_stub
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label auto-implement --label origin:self'
}

@test "explore fallback filing: idempotent label create precedes issue filing" {
    _explore_gh_stub
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'label create origin:self' "$GH_LOG"
}

@test "explore fallback filing: label create failure does not abort filing" {
    _explore_gh_stub
    export GH_LABEL_EXIT=1
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label origin:self'
}

# ── skills/autospec-shared/scripts/grow-define-file-issues.sh ───────────────

_grow_gh_stub() {
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
if [ -n "${GH_LABEL_FAIL:-}" ] && [ "$1" = "label" ]; then exit 1; fi
if [ "$1" = "label" ]; then exit 0; fi
n="$(cat "$GH_COUNTER" 2>/dev/null || echo 200)"; n=$((n+1)); echo "$n" > "$GH_COUNTER"
echo "https://github.com/acme/site/issues/$n"
SH
    chmod +x "$STUB_DIR/gh"
    export GH_COUNTER="$TMP/gh.counter"
}

_grow_ranked() {
    : > "$TMP/r.jsonl"
    echo '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add vs page","norm_title":"add vs page","roi":5,"effort":"small","severity":5,"confidence":0.9,"rank_score":0.9}' >> "$TMP/r.jsonl"
    echo "$TMP/r.jsonl"
}

@test "grow-define filing: every created issue carries origin:self label" {
    _grow_gh_stub
    export GROWTH_LEDGER="$TMP/ledger.jsonl"
    echo '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/cfg.json"
    run bash "$GROW_DEFINE" "$(_grow_ranked)" "$TMP/cfg.json"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label.*origin:self'
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label.*growth:artifact'
}

@test "grow-define filing: idempotent label create precedes issue filing" {
    _grow_gh_stub
    export GROWTH_LEDGER="$TMP/ledger.jsonl"
    echo '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/cfg.json"
    run bash "$GROW_DEFINE" "$(_grow_ranked)" "$TMP/cfg.json"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'label create origin:self' "$GH_LOG"
}

@test "grow-define filing: label create failure does not abort filing" {
    _grow_gh_stub
    export GH_LABEL_FAIL=1
    export GROWTH_LEDGER="$TMP/ledger.jsonl"
    echo '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/cfg.json"
    run bash "$GROW_DEFINE" "$(_grow_ranked)" "$TMP/cfg.json"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label.*origin:self'
    [ "$(grep -c '"outcome":"pending"' "$GROWTH_LEDGER")" -eq 1 ]
}
