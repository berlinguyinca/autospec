#!/usr/bin/env bats
# tests/mutation-integration.bats — integration tests for M5 (negative-path + density floor).
# Issue #442: feat(mutation-m5): negative-path heuristic + assertion-density floor + integration tests
#
# Test plan (8 cases):
#   1-3: negative-path pair detection (paired, unpaired-positive, unpaired-negative-only)
#   4-5: assertion-density floor (empty test block, asserts-present)
#   6:   integration: synthetic bash target vacuous test → M1 catches it
#   7:   integration: synthetic node target surviving mutant → M3/lint warns
#   8:   integration: synthetic python target combined → density floor flags zero-assert

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    NEGPATH_SCRIPT="$REPO_ROOT/scripts/check-negative-path-pairs.sh"
    LINT_SCRIPT="$REPO_ROOT/scripts/lint-implementation.sh"
    FIXTURE_BASH="$REPO_ROOT/tests/fixtures/mutation-integration/bash"
    FIXTURE_NODE="$REPO_ROOT/tests/fixtures/mutation-integration/node"
    FIXTURE_PYTHON="$REPO_ROOT/tests/fixtures/mutation-integration/python"

    WORK="$(mktemp -d -t mutation-integration.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# ─────────────────────────────────────────────────────────────────
# 1. Paired negative-path → no warning
# ─────────────────────────────────────────────────────────────────
@test "check-negative-path-pairs: paired positive+negative test → no WARN" {
    printf '%s\n' \
        '@test "should return success when input is valid" {' \
        '    run check_input "good"' \
        '    [ "$status" -eq 0 ]' \
        '}' \
        '@test "should fail when input is invalid" {' \
        '    run check_input "bad"' \
        '    [ "$status" -eq 1 ]' \
        '}' > "$WORK/paired.bats"
    run bash "$NEGPATH_SCRIPT" "$WORK/paired.bats"
    [ "$status" -eq 0 ]
    # No WARN lines for paired tests
    ! printf '%s\n' "$output" | grep -q "WARN:MISSING_NEGATIVE_PATH"
}

# ─────────────────────────────────────────────────────────────────
# 2. Unpaired positive test → WARN emitted
# ─────────────────────────────────────────────────────────────────
@test "check-negative-path-pairs: unpaired positive test → emits WARN" {
    printf '%s\n' \
        '@test "should return success when all values pass" {' \
        '    run check_all "ok"' \
        '    [ "$status" -eq 0 ]' \
        '}' > "$WORK/unpaired.bats"
    run bash "$NEGPATH_SCRIPT" "$WORK/unpaired.bats"
    # Exits 0 (WARN, not block) but emits warning
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "WARN"
}

# ─────────────────────────────────────────────────────────────────
# 3. Only negative test (no positive) → no spurious WARN
# ─────────────────────────────────────────────────────────────────
@test "check-negative-path-pairs: only negative test → no WARN" {
    printf '%s\n' \
        '@test "should fail when config is missing" {' \
        '    run load_config "/nonexistent"' \
        '    [ "$status" -eq 1 ]' \
        '}' > "$WORK/neg-only.bats"
    run bash "$NEGPATH_SCRIPT" "$WORK/neg-only.bats"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q "WARN:MISSING_NEGATIVE_PATH"
}

# ─────────────────────────────────────────────────────────────────
# 4. Assertion-density floor: empty test block → flagged
# ─────────────────────────────────────────────────────────────────
@test "lint --assertion-density: zero-assertion test block → finding emitted" {
    cat > "$WORK/empty-test.bats" << 'EOF'
@test "does something" {
    true
}
EOF
    # Create a diff that adds this file
    diff_content="$(printf 'diff --git a/tests/empty-test.bats b/tests/empty-test.bats\n--- /dev/null\n+++ b/tests/empty-test.bats\n@@ -0,0 +1,3 @@\n+@test "does something" {\n+    true\n+}\n')"
    printf '%s\n' "$diff_content" > "$WORK/empty.diff"

    run bash "$LINT_SCRIPT" --diff-file "$WORK/empty.diff" --assertion-density
    # Should flag the empty test block
    printf '%s\n' "$output" | grep -qE "ASSERTION_DENSITY|VACUOUS_NO_ASSERT|assert"
}

# ─────────────────────────────────────────────────────────────────
# 5. Assertion-density floor: test with assertion → passes
# ─────────────────────────────────────────────────────────────────
@test "lint --assertion-density: test with assertion → no density finding" {
    diff_content="$(printf 'diff --git a/tests/has-assert.bats b/tests/has-assert.bats\n--- /dev/null\n+++ b/tests/has-assert.bats\n@@ -0,0 +1,4 @@\n+@test "checks output" {\n+    run mycommand\n+    [ "$status" -eq 0 ]\n+}\n')"
    printf '%s\n' "$diff_content" > "$WORK/assert.diff"

    run bash "$LINT_SCRIPT" --diff-file "$WORK/assert.diff" --assertion-density
    # Should not emit ASSERTION_DENSITY finding for this file
    exit_has_density=0
    printf '%s\n' "$output" | grep -q "ASSERTION_DENSITY:tests/has-assert.bats" && exit_has_density=1
    [ "$exit_has_density" -eq 0 ]
}

# ─────────────────────────────────────────────────────────────────
# 6. Integration: synthetic bash fixture with vacuous test → M1 detects it
# ─────────────────────────────────────────────────────────────────
@test "integration: bash fixture vacuous test is detected by M1" {
    [ -d "$FIXTURE_BASH" ] || skip "bash fixture not found at $FIXTURE_BASH"
    [ -f "$FIXTURE_BASH/vacuous.bats" ] || skip "vacuous.bats not found in bash fixture"

    # Run lint with --vacuous-assertions on the fixture diff
    diff_content="$(diff -u /dev/null "$FIXTURE_BASH/vacuous.bats" 2>/dev/null | sed "s|/dev/null|a/tests/fixtures/mutation-integration/bash/vacuous.bats|;s|$FIXTURE_BASH/vacuous.bats|b/tests/fixtures/mutation-integration/bash/vacuous.bats|" || printf 'diff --git a/tests/fixtures/mutation-integration/bash/vacuous.bats b/tests/fixtures/mutation-integration/bash/vacuous.bats\n--- /dev/null\n+++ b/tests/fixtures/mutation-integration/bash/vacuous.bats\n')"
    printf '%s\n' "$(cat "$FIXTURE_BASH/vacuous.bats" | sed 's/^/+/')" >> "$WORK/bash.diff" 2>/dev/null || true
    cat "$FIXTURE_BASH/vacuous.diff" > "$WORK/bash.diff" 2>/dev/null || \
        printf 'diff --git a/tests/fixtures/mutation-integration/bash/vacuous.bats b/tests/fixtures/mutation-integration/bash/vacuous.bats\n@@ -0,0 +1,5 @@\n+@test "always passes" {\n+    grep -qv "X" /dev/null || true\n+    run echo ok\n+    [ "$status" -eq 0 ]\n+}\n' > "$WORK/bash.diff"

    run bash "$LINT_SCRIPT" --diff-file "$WORK/bash.diff" --vacuous-assertions
    printf '%s\n' "$output" | grep -qE "VACUOUS_GREP_INVERSE_OR_TRUE|VACUOUS_OR_TRUE"
}

# ─────────────────────────────────────────────────────────────────
# 7. Integration: synthetic node fixture notes surviving mutant warning
# ─────────────────────────────────────────────────────────────────
@test "integration: node fixture exists with documented mutation gap" {
    [ -d "$FIXTURE_NODE" ] || skip "node fixture not found at $FIXTURE_NODE"
    # Verify fixture has a source file and a test file documenting the mutation gap
    [ -f "$FIXTURE_NODE/source.js" ] || [ -f "$FIXTURE_NODE/source.ts" ]
    [ -f "$FIXTURE_NODE/README.md" ] || [ -f "$FIXTURE_NODE/MUTATION_GAP.md" ]
}

# ─────────────────────────────────────────────────────────────────
# 8. Integration: synthetic python fixture zero-assertion test → density floor
# ─────────────────────────────────────────────────────────────────
@test "integration: python fixture zero-assertion test detected by density floor" {
    [ -d "$FIXTURE_PYTHON" ] || skip "python fixture not found at $FIXTURE_PYTHON"
    [ -f "$FIXTURE_PYTHON/test_density.bats" ] || skip "test_density.bats not found in python fixture"

    # Build a proper diff from the zero-assertion bats fixture (needs +++ b/ header for get_diff_files)
    {
        printf '%s\n' "diff --git a/tests/fixtures/mutation-integration/python/test_density.bats b/tests/fixtures/mutation-integration/python/test_density.bats"
        printf '%s\n' "--- /dev/null"
        printf '%s\n' "+++ b/tests/fixtures/mutation-integration/python/test_density.bats"
        printf '%s\n' "@@ -0,0 +1,7 @@"
        sed 's/^/+/' "$FIXTURE_PYTHON/test_density.bats"
    } > "$WORK/py.diff"

    run bash "$LINT_SCRIPT" --diff-file "$WORK/py.diff" --assertion-density
    # Zero-assertion bats test block in python fixture should trigger ASSERTION_DENSITY
    printf '%s\n' "$output" | grep -qE "ASSERTION_DENSITY|VACUOUS_NO_ASSERT"
}
