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
    export GITHUB_REPOSITORY="berlinguyinca/autospec"
    cat > "$STUB_DIR/autospec" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
if [ "${1:-}" != "queue" ] || [ "${2:-}" != "review-safety" ]; then
  if [ "${1:-}" = "project" ] && [ "${2:-}" = "sync" ]; then
    exit 0
  fi
  exit 41
fi
printf '%s\n' '{"pass":1,"ambiguous":0,"block":0,"stale":0,"conflicted":0,"skipped":0}'
SH
    chmod +x "$STUB_DIR/autospec"
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
REPO_ROOT="$TMP"
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
$(awk '/^project_sync_issue\(\)/,/^}/' "$EXPLORE")
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

@test "explore fallback filing: Rust reviews the exact issue after creation" {
    _explore_gh_stub
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'queue review-safety --repo berlinguyinca/autospec --limit 1 --issue 321' "$GH_LOG"
    create_line="$(grep -n 'issue create' "$GH_LOG" | head -1 | cut -d: -f1)"
    review_line="$(grep -n 'queue review-safety' "$GH_LOG" | head -1 | cut -d: -f1)"
    [ "$create_line" -lt "$review_line" ]
}

@test "explore fallback filing: label create failure does not abort filing" {
    _explore_gh_stub
    export GH_LABEL_EXIT=1
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label origin:self'
}

@test "explore fallback filing: stderr-visible retry also carries --label origin:self" {
    # Primary issue create fails (exit 1, no URL); the retry succeeds. Both
    # invocations must carry origin:self — the retry is a distinct call site.
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit 0 ;;
  *"issue create"*)
    n="$(cat "$GH_CREATE_COUNT" 2>/dev/null || echo 0)"; n=$((n+1)); echo "$n" > "$GH_CREATE_COUNT"
    if [ "$n" -eq 1 ]; then echo "gh: transient failure" >&2; exit 1; fi
    printf 'https://github.com/berlinguyinca/autospec/issues/322\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$STUB_DIR/gh"
    export GH_CREATE_COUNT="$TMP/gh.create.count"
    run _run_explore_raw_filing
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    # Both the failed primary and the successful retry were invoked.
    [ "$(cat "$GH_CREATE_COUNT")" -eq 2 ]
    # Two labeled issue-create invocations in the log (primary + retry).
    [ "$(tr '\n' ' ' < "$GH_LOG" | grep -o -- '--label auto-implement --label origin:self' | wc -l | tr -d ' ')" -eq 2 ]
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
