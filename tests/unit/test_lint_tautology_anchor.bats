#!/usr/bin/env bats
# tests/unit/test_lint_tautology_anchor.bats — VACUOUS_TAUTOLOGY must match the
# Jest/Mocha skip function xit without also matching identifiers that end in "xit".
#
# Unanchored, the pattern matched sys.exit( and raise SystemExit(, so every changed
# Python or JavaScript file with an exit call reported "Tautological assertion" — a
# finding that cannot be acted on, and one the documented linter:allow hatch does not
# reach. Lives in its own file because tests/unit/test_lint_implementation.bats is
# past the file-size limit and may not grow.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "${TMP:?}"
}

# Builds a one-file diff whose added lines are the arguments.
vac_diff() {
    local path="$1"; shift
    {
        printf 'diff --git a/%s b/%s\n--- /dev/null\n+++ b/%s\n@@ -0,0 +1,%s @@\n' \
            "$path" "$path" "$path" "$#"
        for line in "$@"; do printf '+%s\n' "$line"; done
    } > "$TMP/change.diff"
}

@test "tautology anchor: sys.exit is not a tautological assertion" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'scripts/tool.py' 'if not profile:' '    sys.exit(1)'
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    ! printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}

@test "tautology anchor: raise SystemExit is not a tautological assertion" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'scripts/tool.py' '    raise SystemExit(main())'
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    ! printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}

@test "tautology anchor: an identifier ending in xit is not flagged" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'src/app.js' '  const code = graceful_exit(1);'
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    ! printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}

@test "tautology anchor: a real xit skip at line start is still flagged" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'tests/unit/test_x.js' "xit('pending', () => {" '  expect(1).toBe(1);' '});'
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}

@test "tautology anchor: a real xit after leading whitespace is still flagged" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'tests/unit/test_x.js' "  xit('pending', () => { expect(a).toBe(b); });"
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}

@test "tautology anchor: other tautology forms still match" {
    # linter:allow-VACUOUS_TAUTOLOGY fixture text for the rule under test
    vac_diff 'tests/unit/test_y.js' '  expect(true).toBe(true);'
    run bash "$LINT" --diff-file "$TMP/change.diff" --vacuous-assertions
    printf '%s\n' "$output" | grep -q 'VACUOUS_TAUTOLOGY'
}
