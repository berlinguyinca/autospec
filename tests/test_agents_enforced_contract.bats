#!/usr/bin/env bats
# tests/test_agents_enforced_contract.bats — regression for feat(agents-md) #807
#
# Verifies:
# - AGENTS.md has an ## Enforcement section with linter:allow- syntax
# - lint-implementation.sh has an is_line_allowed function
# - At least 3 rules are documented as deterministically enforced

AGENTS_MD="${BATS_TEST_DIRNAME}/../AGENTS.md"
LINT_SH="${BATS_TEST_DIRNAME}/../scripts/lint-implementation.sh"

@test "AGENTS.md contains an Enforcement section" {
    run grep -c "^### Enforcement" "$AGENTS_MD"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AGENTS.md documents linter:allow- escape hatch syntax" {
    run grep -c "linter:allow-" "$AGENTS_MD"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AGENTS.md Enforcement section documents at least 3 deterministic rules" {
    # Count rows in the enforcement table (lines with | MISSING_TEST|MOCK_DB|SECURITY)
    run grep -cE "MISSING_TEST|MOCK_DB|SECURITY" "$AGENTS_MD"
    [ "$status" -eq 0 ]
    [ "$output" -ge 3 ]
}

@test "lint-implementation.sh contains is_line_allowed function for escape hatch" {
    run grep -c "^is_line_allowed()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "is_line_allowed checks for linter:allow- pattern" {
    run grep -c "linter:allow-" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "escape hatch requires a reason (bare linter:allow-X rejected)" {
    # The pattern in is_line_allowed requires a non-space after the rule ID
    run grep -c "linter:allow-.*\[^\[:space:\]\]" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}
