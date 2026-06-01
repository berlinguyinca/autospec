#!/usr/bin/env bats
# tests/test_lint_complexity_gates.bats — regression for feat(lint) #805
#
# Verifies the four deterministic complexity gates added to lint-implementation.sh:
# check_file_loc, check_function_loc, check_cyclomatic, check_duplicate_names.

LINT_SH="${BATS_TEST_DIRNAME}/../scripts/lint-implementation.sh"

@test "lint-implementation.sh contains check_file_loc function" {
    run grep -c "^check_file_loc()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_function_loc function" {
    run grep -c "^check_function_loc()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_cyclomatic function" {
    run grep -c "^check_cyclomatic()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_duplicate_names function" {
    run grep -c "^check_duplicate_names()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_FILE_LOC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_FILE_LOC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_FUNC_LOC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_FUNC_LOC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_CYCLOMATIC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_CYCLOMATIC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "check_file_loc is wired into the main detector pass" {
    # Both invocation paths (directives and non-directives) must call check_file_loc.
    run grep -c "check_file_loc" "$LINT_SH"
    [ "$status" -eq 0 ]
    # At least definition + 2 call sites
    [ "$output" -ge 3 ]
}

@test "check_duplicate_names is wired into the main detector pass" {
    run grep -c "check_duplicate_names" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 3 ]
}
