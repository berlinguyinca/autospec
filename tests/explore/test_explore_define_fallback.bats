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
    export PATH="$TMP/bin:$PATH"
}

teardown() {
    cd /
    rm -rf "$TMP"
}

_run_filing() {
    cat > "$TMP/driver.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
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
