#!/usr/bin/env bats
# tests/unit/test_lint_implementation_shell_asserts.bats — shell test-expression assertions.
# Regression tests: POSIX [ ... ] and bash [[ ... ]] test expressions count as
# assertions for ASSERTION_DENSITY / VACUOUS_NO_ASSERT even when the block uses
# no assert/expect/run/grep/check/verify keywords.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
}

_vac_diff() {
    local fpath="$1"; shift
    _vac_tmpfile="$(mktemp -t lint-vac-XXXXXX.diff)"
    {
        printf 'diff --git a/%s b/%s\nnew file mode 100644\n--- /dev/null\n+++ b/%s\n@@ -0,0 +1,%s @@\n' \
            "$fpath" "$fpath" "$fpath" "$#"
        for line in "$@"; do
            printf '+%s\n' "$line"
        done
    } > "$_vac_tmpfile"
}

@test "density: no ASSERTION_DENSITY for bats test using only [ ... ] test expressions" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "reference file is non-empty" {' \
        '  [ -f "$REF" ]' \
        '  [ -s "$REF" ]' \
        '  [ "$(wc -l < "$REF")" -gt 100 ]' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --assertion-density
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "ASSERTION_DENSITY"
}

@test "vacuous: no VACUOUS_NO_ASSERT for bats test using only [[ ... ]] test expressions" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "output is set" {' \
        '  [[ -n "$output" ]]' \
        '  [[ "$output" == *expected* ]]' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_NO_ASSERT"
}

@test "density: ASSERTION_DENSITY still fires for JS test whose [ ... ] line is an array literal" {
    _vac_diff 'tests/unit/test_x.js' \
        'it("lists items", () => {' \
        '  [ a, b ].forEach(render)' \
        '  render(a)' \
        '});'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --assertion-density
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "ASSERTION_DENSITY"
}
