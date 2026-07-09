#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-issue-safety.sh"
    FIX="$REPO_ROOT/tests/fixtures/issue-safety"
}

@test "lint-issue-safety: safe docs change passes" {
    run bash "$LINT" --title "Update config docs" "$FIX/safe-docs-change.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
}

@test "lint-issue-safety: production deletion blocks" {
    run bash "$LINT" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: vague data cleanup quarantines as ambiguous" {
    run bash "$LINT" --title "Clean old data" "$FIX/ambiguous-clean-data.md"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "SAFETY_AMBIGUOUS"
    echo "$output" | grep -q "vague-data-cleanup"
}

@test "lint-issue-safety: trusted actor can reset test database" {
    run bash "$LINT" --actor berlinguyinca --title "Reset test database" "$FIX/trusted-test-db-reset.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
    echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: trusted actor cannot dump secrets" {
    run bash "$LINT" --actor berlinguyinca --title "Dump secrets" "$FIX/trusted-secret-dump.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "secret-exfiltration"
}

@test "lint-issue-safety: invalid YAML falls back to defaults and blocks dangerous body" {
    run bash "$LINT" --config "$FIX/invalid-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: json mode emits decision field" {
    run bash "$LINT" --json --title "Clean old data" "$FIX/ambiguous-clean-data.md"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q '"decision":"SAFETY_AMBIGUOUS"'
    echo "$output" | grep -q '"rule_id":"vague-data-cleanup"'
}
