#!/usr/bin/env bats
# tests/unit/test_validate_workflow.bats — assert that the validate.yml
# CI workflow exists, parses as YAML, and contains the required triggers
# and steps.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.6.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    WORKFLOW="$REPO_ROOT/.github/workflows/validate.yml"
}

@test ".github/workflows/validate.yml exists" {
    [ -f "$WORKFLOW" ]
}

@test "validate.yml is valid YAML" {
    if ! command -v python3 >/dev/null 2>&1; then
        skip "python3 not available"
    fi
    run python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml triggers on push" {
    run grep -q '^  *push:' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml triggers on pull_request" {
    run grep -q '^  *pull_request:' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml runs scripts/dev-bootstrap.sh" {
    run grep -q 'scripts/dev-bootstrap.sh' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml runs scripts/validate.sh" {
    run grep -q 'scripts/validate.sh' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml runs bats tests/unit tests/smoke" {
    run grep -q 'bats tests/unit tests/smoke' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml uses ubuntu-latest" {
    run grep -q 'runs-on: ubuntu-latest' "$WORKFLOW"
    [ "$status" -eq 0 ]
}

@test "validate.yml uses actions/checkout@v4" {
    run grep -q 'actions/checkout@v4' "$WORKFLOW"
    [ "$status" -eq 0 ]
}
