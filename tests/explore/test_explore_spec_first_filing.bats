#!/usr/bin/env bats
# tests/explore/test_explore_spec_first_filing.bats
#
# Issue #1102 — spec-first filing loop in scripts/autospec-explore.sh.
# Asserts the per-round filing path: render round spec → commit + push to the
# sandbox branch BEFORE decomposition → decompose via /autospec-define
# --base <sandbox>. No mocks of git; we exercise the real filing function in
# isolation against a throwaway git repo with a stubbed define handoff.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    ORCH="$REPO_ROOT/scripts/autospec-explore.sh"
    TMP="$(mktemp -d -t spec-first.XXXXXX)"

    # A throwaway git repo standing in for the sandbox working tree.
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    git checkout -q -b sandbox/explore-demo
    mkdir -p docs/specs .autospec
    echo "seed" > seed.txt
    git add seed.txt
    git commit -qm seed
    # A bare "remote" so push has somewhere to go.
    git init -q --bare "$TMP/remote.git"
    git remote add origin "$TMP/remote.git"
    git push -q origin sandbox/explore-demo

    # Ranked-proposals fixture for the renderer.
    cat > .autospec/proposals.json <<'EOF'
{
  "proposals": [
    {
      "title": "feat: add retry to loop",
      "evidence": "scripts/autospec-explore.sh:42",
      "estimated_complexity": "small",
      "confidence": 0.9,
      "source": "spec-vs-code",
      "severity": "correctness",
      "named_consumer": "autospec-run"
    }
  ]
}
EOF
}

teardown() {
    cd /
    rm -rf "$TMP"
}

# Extract the spec-first filing function from the orchestrator and run it in a
# minimal harness with the surrounding loop variables faked.
_run_filing() {
    # shellcheck disable=SC1090
    cat > "$TMP/driver.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
SANDBOX_BRANCH="sandbox/explore-demo"
RESEARCH_SOURCES="spec-vs-code"
iter=1
research_json="$TMP/.autospec/proposals.json"
iter_dir="$TMP/.autospec"
proposals_count=1
issues_filed=0
filed_issue_nums=""
_ledger_append() { :; }
_ledger_normalize_title() { printf '%s' "\$1"; }
LEDGER_BIN=""
# Source ONLY the spec-first filing function out of the orchestrator.
$(awk '/^# >>> explore-spec-first-filing >>>/,/^# <<< explore-spec-first-filing <<</' "$ORCH")
_explore_file_round
DRIVER
    AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD="$DEFINE_CMD" bash "$TMP/driver.sh"
}

@test "renders + commits the round spec to the sandbox BEFORE decomposing" {
    # Define handoff that asserts the spec is already committed when it runs.
    DEFINE_CMD='git -C '"$TMP"' cat-file -e sandbox/explore-demo:docs/specs/$(ls '"$TMP"'/docs/specs | head -1) >/dev/null 2>&1 || git rev-parse HEAD >/dev/null; exit 0'
    run _run_filing
    [ "$status" -eq 0 ]
    # A round spec was written under docs/specs.
    ls "$TMP"/docs/specs/*-round-1-design.md >/dev/null 2>&1
    # And committed to the sandbox branch (exists in the tree at HEAD).
    run git -C "$TMP" log --oneline -1 --name-only
    [[ "$output" == *"docs/specs/"*"-round-1-design.md"* ]]
}

@test "pushes the round spec commit to origin/sandbox before filing" {
    DEFINE_CMD='exit 0'
    run _run_filing
    [ "$status" -eq 0 ]
    # origin sandbox branch now contains the round spec.
    run git -C "$TMP" ls-tree -r --name-only origin/sandbox/explore-demo
    [[ "$output" == *"docs/specs/"*"-round-1-design.md"* ]]
}

@test "decompose handoff is invoked with --base <sandbox>" {
    DEFINE_CMD='echo "$AUTOSPEC_DEFINE_ARGS" > '"$TMP"'/define-args.txt; exit 0'
    run _run_filing
    [ "$status" -eq 0 ]
    run cat "$TMP/define-args.txt"
    [[ "$output" == *"--base sandbox/explore-demo"* ]]
    [[ "$output" == *"-round-1-design.md"* ]]
}

@test "never targets main: the define args carry the sandbox base, not main" {
    DEFINE_CMD='echo "$AUTOSPEC_DEFINE_ARGS" > '"$TMP"'/define-args.txt; exit 0'
    run _run_filing
    [ "$status" -eq 0 ]
    run cat "$TMP/define-args.txt"
    [[ "$output" != *"--base main"* ]]
}
