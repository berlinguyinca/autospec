#!/usr/bin/env bats
# tests/unit/test_e2e_workflow.bats — assert that the e2e.yml CI workflow
# exists, parses as YAML, is gated on the `e2e` PR label or
# workflow_dispatch, and runs `bats tests/e2e`.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.6.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    WORKFLOW="$REPO_ROOT/.github/workflows/e2e.yml"
}

@test ".github/workflows/e2e.yml exists" {
    [ -f "$WORKFLOW" ]
}

@test "e2e.yml is valid YAML" {
    if ! command -v python3 >/dev/null 2>&1; then
        skip "python3 not available"
    fi
    run python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml triggers on pull_request labeled" {
    run grep -q 'pull_request:' "$WORKFLOW"
    [ "$status" -eq 0 ]
    run grep -q 'types:' "$WORKFLOW"
    [ "$status" -eq 0 ]
    run grep -qE '\blabeled\b' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml triggers on workflow_dispatch" {
    run grep -q 'workflow_dispatch:' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml gates the job on the 'e2e' label" {
    run grep -q "label.name == 'e2e'" "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml runs bats tests/e2e" {
    run grep -q 'bats tests/e2e' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml uses ubuntu-latest" {
    run grep -q 'runs-on: ubuntu-latest' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml exposes GH_TOKEN to the run step" {
    run grep -q 'GH_TOKEN' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "e2e.yml has a 5-minute timeout" {
    run grep -q 'timeout-minutes: 5' "$WORKFLOW"
    [ "$status" -eq 0 ]
}
