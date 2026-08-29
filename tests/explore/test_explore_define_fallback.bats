#!/usr/bin/env bats
# tests/explore/test_explore_define_fallback.bats
#
# Issue #1102 — fallback path: when the /autospec-define handoff is unavailable
# or exits non-zero, autospec-explore logs code_health:explore_define_unavailable,
# STILL commits the round spec, falls back to raw `gh issue create` filing for
# that round, and continues. Uses a fake `gh` on PATH (no network).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    ORCH="$REPO_ROOT/scripts/autospec-explore.sh"
    TMP="$(mktemp -d -t define-fallback.XXXXXX)"

    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    git checkout -q -b sandbox/explore-demo
    mkdir -p docs/specs .autospec bin
    echo seed > seed.txt
    git add seed.txt && git commit -qm seed
    git init -q --bare "$TMP/remote.git"
    git remote add origin "$TMP/remote.git"
    git push -q origin sandbox/explore-demo

    cat > .autospec/proposals.json <<'EOF'
{
  "proposals": [
    { "title": "fix: handle empty research", "evidence": "x:1",
      "estimated_complexity": "medium", "confidence": 0.6,
      "source": "codebase-signals", "severity": "stability",
      "named_consumer": "autospec-explore" }
  ]
}
EOF

    # Fake gh that records issue-create calls and emits a fake URL.
    cat > bin/gh <<EOF
#!/usr/bin/env bash
if [ "\$1" = "issue" ] && [ "\$2" = "create" ]; then
  echo "issue:create \$*" >> "$TMP/gh.log"
  echo "https://github.com/o/r/issues/9001"
  exit 0
fi
exit 0
EOF
    chmod +x bin/gh
cat > bin/autospec <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/safety.log"
if [ "\${1:-}" = "project" ] && [ "\${2:-}" = "sync" ]; then
  exit 0
fi
if [ "\${1:-}" != "queue" ] || [ "\${2:-}" != "review-safety" ]; then
  exit 41
fi
printf '%s\n' '{"pass":1,"ambiguous":0,"block":0,"stale":0,"conflicted":0,"skipped":0}'
EOF
    chmod +x bin/autospec
    export PATH="$TMP/bin:$PATH"
    export GITHUB_REPOSITORY="o/r"
}

teardown() {
    cd /
    rm -rf "$TMP"
}

_run_filing() {
    cat > "$TMP/driver.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
REPO_ROOT="$TMP"
SANDBOX_BRANCH="sandbox/explore-demo"
RESEARCH_SOURCES="codebase-signals"
iter=1
research_json="$TMP/.autospec/proposals.json"
iter_dir="$TMP/.autospec"
proposals_count=1
issues_filed=0
filed_issue_nums=""
_ledger_append() { :; }
_ledger_normalize_title() { printf '%s' "\$1"; }
LEDGER_BIN=""
$(awk '/^project_sync_issue\(\)/,/^}/' "$ORCH")
$(awk '/^# >>> explore-spec-first-filing >>>/,/^# <<< explore-spec-first-filing <<</' "$ORCH")
_explore_file_round
DRIVER
    AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD="$DEFINE_CMD" bash "$TMP/driver.sh" 2>"$TMP/err.log"
}

@test "non-zero define exit logs explore_define_unavailable" {
    DEFINE_CMD='exit 7'
    run _run_filing
    [ "$status" -eq 0 ]
    grep -q 'code_health:explore_define_unavailable' "$TMP/err.log"
}

@test "non-zero define still commits the round spec" {
    DEFINE_CMD='exit 7'
    run _run_filing
    [ "$status" -eq 0 ]
    run git -C "$TMP" log --oneline -1 --name-only
    [[ "$output" == *"docs/specs/"*"-round-1-design.md"* ]]
}

@test "non-zero define falls back to raw gh issue create" {
    DEFINE_CMD='exit 7'
    run _run_filing
    [ "$status" -eq 0 ]
    grep -q 'issue:create' "$TMP/gh.log"
    grep -q 'auto-implement' "$TMP/gh.log"
}

@test "raw fallback reviews the newly created issue through Rust" {
    DEFINE_CMD='exit 7'
    run _run_filing
    [ "$status" -eq 0 ]
    grep -q 'queue review-safety --repo o/r --limit 1 --issue 9001' "$TMP/safety.log"
}

@test "missing define handoff (empty cmd) also falls back" {
    DEFINE_CMD=''
    run _run_filing
    [ "$status" -eq 0 ]
    grep -q 'code_health:explore_define_unavailable' "$TMP/err.log"
    grep -q 'issue:create' "$TMP/gh.log"
}

@test "successful define does NOT fall back to raw filing" {
    DEFINE_CMD='exit 0'
    run _run_filing
    [ "$status" -eq 0 ]
    [ ! -f "$TMP/gh.log" ]
    ! grep -q 'code_health:explore_define_unavailable' "$TMP/err.log"
}

# ── issues_filed on fallback path: raw filing counter only (#1109) ───────────

_run_filing_with_counter() {
    cat > "$TMP/driver_counter.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
REPO_ROOT="$TMP"
SANDBOX_BRANCH="sandbox/explore-demo"
RESEARCH_SOURCES="codebase-signals"
iter=1
research_json="$TMP/.autospec/proposals.json"
iter_dir="$TMP/.autospec"
proposals_count=1
issues_filed=0
filed_issue_nums=""
_ledger_append() { :; }
_ledger_normalize_title() { printf '%s' "\$1"; }
LEDGER_BIN=""
$(awk '/^project_sync_issue\(\)/,/^}/' "$ORCH")
$(awk '/^# >>> explore-spec-first-filing >>>/,/^# <<< explore-spec-first-filing <<</' "$ORCH")
_explore_file_round
printf 'issues_filed=%s\n' "\$issues_filed"
DRIVER
    AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD="$DEFINE_CMD" bash "$TMP/driver_counter.sh" 2>"$TMP/err2.log"
}

@test "fallback path: issues_filed counts raw-filed issues, not snapshot diff" {
    # gh stub: list returns nothing (so snapshot diff would give 0), but
    # issue create returns a URL — raw filing should still count it.
    cat > bin/gh <<'GHSTUB'
#!/usr/bin/env bash
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    # empty — would make snapshot diff yield 0
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
    echo "https://github.com/o/r/issues/9002"
    exit 0
fi
exit 0
GHSTUB
    chmod +x bin/gh

    DEFINE_CMD='exit 7'
    run _run_filing_with_counter
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    # Raw filing incremented issues_filed for the one proposal.
    [[ "$output" == *"issues_filed=1"* ]]
}
