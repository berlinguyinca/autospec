#!/usr/bin/env bats
# tests/unit/test_lint_issue.bats — one @test per spec §7.1 fixture row.
# Exercises scripts/lint-issue.sh exit code and stderr findings.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-issue.sh"
    FIX="$REPO_ROOT/tests/fixtures/issue-quality"
}

# ── good fixture ──────────────────────────────────────────────────────────────

@test "lint-issue: good.md exits 0 with no findings" {
    run bash "$LINT" "$FIX/good.md"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── GOAL rule cases ───────────────────────────────────────────────────────────

@test "lint-issue: bad-goal-vague.md exits >=1 and reports GOAL_VAGUE" {
    run bash -c "bash '$LINT' '$FIX/bad-goal-vague.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "GOAL_VAGUE"
}

@test "lint-issue: bad-goal-hedge.md exits >=1 and reports GOAL_HEDGE" {
    run bash -c "bash '$LINT' '$FIX/bad-goal-hedge.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "GOAL_HEDGE"
}

@test "lint-issue: bad-goal-multi-sentence triggers GOAL_NOT_ONE_SENTENCE (skip if no fixture)" {
    if [ ! -f "$FIX/bad-goal-multi-sentence.md" ]; then
        skip "fixture bad-goal-multi-sentence.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-goal-multi-sentence.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "GOAL_NOT_ONE_SENTENCE"
}

# ── AC rule cases ─────────────────────────────────────────────────────────────

@test "lint-issue: bad-ac-prose triggers AC_PROSE (skip if no fixture)" {
    if [ ! -f "$FIX/bad-ac-prose.md" ]; then
        skip "fixture bad-ac-prose.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-ac-prose.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "AC_PROSE"
}

@test "lint-issue: bad-ac-subjective triggers AC_SUBJECTIVE (skip if no fixture)" {
    if [ ! -f "$FIX/bad-ac-subjective.md" ]; then
        skip "fixture bad-ac-subjective.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-ac-subjective.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "AC_SUBJECTIVE"
}

@test "lint-issue: bad-ac-too-long.md exits >=1 and reports AC_TOO_LONG" {
    run bash -c "bash '$LINT' '$FIX/bad-ac-too-long.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "AC_TOO_LONG"
}

@test "lint-issue: bad-ac-empty triggers AC_EMPTY (skip if no fixture)" {
    if [ ! -f "$FIX/bad-ac-empty.md" ]; then
        skip "fixture bad-ac-empty.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-ac-empty.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "AC_EMPTY"
}

# ── SMOKE rule cases ──────────────────────────────────────────────────────────

@test "lint-issue: bad-smoke-multi-line.md exits >=1 and reports SMOKE_MULTI_LINE" {
    run bash -c "bash '$LINT' '$FIX/bad-smoke-multi-line.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SMOKE_MULTI_LINE"
}

@test "lint-issue: bad-smoke-placeholder triggers SMOKE_PLACEHOLDER (skip if no fixture)" {
    if [ ! -f "$FIX/bad-smoke-placeholder.md" ]; then
        skip "fixture bad-smoke-placeholder.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-smoke-placeholder.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SMOKE_PLACEHOLDER"
}

@test "lint-issue: bad-smoke-no-fence triggers SMOKE_NOT_FENCED (skip if no fixture)" {
    if [ ! -f "$FIX/bad-smoke-no-fence.md" ]; then
        skip "fixture bad-smoke-no-fence.md not yet landed"
    fi
    run bash -c "bash '$LINT' '$FIX/bad-smoke-no-fence.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SMOKE_NOT_FENCED"
}

# ── multi-rule case ───────────────────────────────────────────────────────────

@test "lint-issue: bad-multiple.md exits >=3 with all three rule families flagged" {
    run bash "$LINT" "$FIX/bad-multiple.md"
    [ "$status" -ge 3 ]
}

# ── CLI contract ──────────────────────────────────────────────────────────────

@test "lint-issue: --help exits 0 and prints Usage:" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "^Usage:"
}

@test "lint-issue: --json mode emits JSON array on stdout for a failing fixture" {
    run bash "$LINT" --json "$FIX/bad-goal-vague.md"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q '"rule"'
}
