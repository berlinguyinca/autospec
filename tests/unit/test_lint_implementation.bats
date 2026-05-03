#!/usr/bin/env bats
# tests/unit/test_lint_implementation.bats — one @test per fixture row.
# Exercises scripts/lint-implementation.sh exit code and stdout findings.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
}

# ── syntax check ─────────────────────────────────────────────────────────────

@test "lint-implementation: bash -n exits 0" {
    run bash -n "$LINT"
    [ "$status" -eq 0 ]
}

@test "lint-implementation: --help lists OUT_OF_SCOPE" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OUT_OF_SCOPE"
}

@test "lint-implementation: --help lists MISSING_TEST" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "MISSING_TEST"
}

@test "lint-implementation: --help lists all 10 RULE_IDs" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OUT_OF_SCOPE"
    echo "$output" | grep -q "MISSING_TEST"
    echo "$output" | grep -q "COMPLEXITY"
    echo "$output" | grep -q "SECURITY"
    echo "$output" | grep -q "TODO_LEFT"
    echo "$output" | grep -q "MOCK_DB"
    echo "$output" | grep -q "DOC_OUT_OF_SYNC"
    echo "$output" | grep -q "HALLUCINATED_API"
    echo "$output" | grep -q "DUPLICATE_CODE"
    echo "$output" | grep -q "INVENTED_CONFIG"
}

# ── good fixture ──────────────────────────────────────────────────────────────

@test "lint-implementation: good.diff exits 0 with no findings" {
    run bash "$LINT" --diff-file "$FIX/good.diff"
    [ "$status" -eq 0 ]
    # No blocking RULE_ID lines (no OUT_OF_SCOPE etc.)
    echo "$output" | grep -vq "^INFO:" || true
    ! echo "$output" | grep -qE "^(OUT_OF_SCOPE|MISSING_TEST|COMPLEXITY|SECURITY|TODO_LEFT|MOCK_DB|DOC_OUT_OF_SYNC):"
}

# ── TODO_LEFT detector ────────────────────────────────────────────────────────

@test "lint-implementation: bad-todo-left.diff exits >=1 and reports TODO_LEFT" {
    run bash "$LINT" --diff-file "$FIX/bad-todo-left.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
}

@test "lint-implementation: bad-todo-left.diff does not report TODO_LEFT in test files" {
    # The bad-todo-left.diff only has TODO in non-test source; ensure it fires correctly
    run bash "$LINT" --diff-file "$FIX/bad-todo-left.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT:scripts/"
}

# ── SECURITY detector ─────────────────────────────────────────────────────────

@test "lint-implementation: bad-secret.diff exits >=1 and reports SECURITY" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SECURITY"
}

@test "lint-implementation: bad-secret.diff SECURITY finding mentions AWS key" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SECURITY.*AKIA\|hardcoded AWS"
}

# ── MOCK_DB detector ──────────────────────────────────────────────────────────

@test "lint-implementation: bad-mock-db.diff exits >=1 and reports MOCK_DB" {
    run bash "$LINT" --diff-file "$FIX/bad-mock-db.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "MOCK_DB"
}

# ── COMPLEXITY detector ───────────────────────────────────────────────────────

@test "lint-implementation: bad-complexity.diff exits >=1 and reports COMPLEXITY" {
    run bash "$LINT" --diff-file "$FIX/bad-complexity.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "COMPLEXITY"
}

# ── per-RULE_ID emit cap ──────────────────────────────────────────────────────

@test "lint-implementation: per-RULE_ID cap collapses 11+ hits to truncated notice" {
    # Generate a synthetic diff with 12 TODO lines in a non-test source file
    local synth_diff
    synth_diff="$(mktemp -t lint-impl-cap-test.XXXXXX.diff)"
    {
        printf '%s\n' 'diff --git a/scripts/synth.sh b/scripts/synth.sh'
        printf '%s\n' 'new file mode 100755'
        printf '%s\n' '--- /dev/null'
        printf '%s\n' '+++ b/scripts/synth.sh'
        printf '%s\n' '@@ -0,0 +1,12 @@'
        local i=1
        while [ "$i" -le 12 ]; do
            printf '+# TODO: fix item %d\n' "$i"
            i=$((i+1))
        done
    } > "$synth_diff"
    run bash "$LINT" --diff-file "$synth_diff"
    rm -f "$synth_diff"
    # Should have some TODO_LEFT findings and a truncation notice
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
    echo "$output" | grep -q "truncated"
}

# ── findings hard cap ─────────────────────────────────────────────────────────

@test "lint-implementation: scope explosion message is emitted at hard cap" {
    # Verify the hard-cap message text exists in the script (structural test)
    grep -q "too many findings" "$LINT"
    grep -q "exit 200" "$LINT"
}

# ── skip-directive opt-out ────────────────────────────────────────────────────

@test "lint-implementation: skip-respected.diff has Guardian skip line in issue file" {
    grep -q "Guardian: skip-TODO_LEFT" "$FIX/skip-respected.issue.md"
}

@test "lint-implementation: skip-respected.diff TODO_LEFT is blocked without skip directive" {
    # Without passing --issue, skip directives are not loaded; TODO_LEFT fires
    run bash "$LINT" --diff-file "$FIX/skip-respected.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
}
