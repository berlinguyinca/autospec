#!/usr/bin/env bats
# ensure-tool-scanners.bats — the security scanners are present in the baked-in table.

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    ENSURE="${SCRIPT_DIR}/scripts/ensure-tool.sh"
}

@test "gitleaks is a known tool (not the unknown no-op path)" {
    run grep -E '^\s+gitleaks\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "semgrep is a known tool" {
    run grep -E '^\s+semgrep\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "trivy is a known tool" {
    run grep -E '^\s+trivy\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "license-checker is a known tool" {
    run grep -E '^\s+license-checker\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "already-present scanner is a no-op exit 0" {
    run bash "$ENSURE" jq
    [ "$status" -eq 0 ]
}
