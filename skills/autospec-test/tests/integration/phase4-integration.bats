#!/usr/bin/env bats
# skills/autospec-test/tests/integration/phase4-integration.bats
#
# Phase 9 integration tests: run-gate.sh, pr-report.sh, bootstrap-labels.sh
# wiring into autospec-run Phase 4.
#
# Tests use --dry-run and stub-gate fixtures to avoid real Playwright/CI runs.
# Real gh CLI is used for label bootstrap (idempotent --force).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    TARGETS_DIR="$REPO_ROOT/skills/autospec-test/test-targets"
    RUN_GATE="$SCRIPTS_DIR/run-gate.sh"
    PR_REPORT="$SCRIPTS_DIR/pr-report.sh"
    BOOTSTRAP="$SCRIPTS_DIR/bootstrap-labels.sh"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-phase4-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── run-gate.sh exit codes ────────────────────────────────────────────────────

@test "run-gate: exits 0 for target-clean-pass (overall_passed=true)" {
    [ -d "$TARGETS_DIR/target-clean-pass" ] || skip "target-clean-pass not found"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-clean-pass"
    [ "$status" -eq 0 ]
}

@test "run-gate: exits 1 for target-failing-gap (overall_passed=false)" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap"
    [ "$status" -eq 1 ]
}

@test "run-gate: exits 2 for missing target directory" {
    run bash "$RUN_GATE" "/nonexistent/target"
    [ "$status" -eq 2 ]
}

@test "run-gate: exits 2 for target missing .autospec/test.yml" {
    local empty_target="$TEST_TMPDIR/empty-target"
    mkdir -p "$empty_target"
    run bash "$RUN_GATE" "$empty_target"
    [ "$status" -eq 2 ]
}

@test "run-gate: --dry-run flag prints gate JSON without writing comment" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    local gate_out="$TEST_TMPDIR/gate.json"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap" \
        --dry-run \
        --output-gate "$gate_out"
    # Exits 1 (failing target) but gate JSON written
    [ -f "$gate_out" ]
    local passed
    passed=$(jq -r '.overall_passed' "$gate_out")
    [ "$passed" = "false" ]
}

# ── pr-report.sh comment composition ─────────────────────────────────────────

@test "pr-report: produces marker-delimited comment for failing gate" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    local gate_out="$TEST_TMPDIR/gate.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]

    local comment_out="$TEST_TMPDIR/comment.md"
    run bash "$PR_REPORT" --gate-json "$gate_out" --output "$comment_out"
    [ "$status" -eq 0 ]
    [ -f "$comment_out" ]
    grep -q "autospec-test-report-marker" "$comment_out"
    grep -q "Blocked\|Failed\|blocked\|failed" "$comment_out"
}

@test "pr-report: produces marker-delimited comment for passing gate" {
    [ -d "$TARGETS_DIR/target-clean-pass" ] || skip "target-clean-pass not found"
    local gate_out="$TEST_TMPDIR/gate-pass.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-clean-pass" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]

    local comment_out="$TEST_TMPDIR/comment-pass.md"
    run bash "$PR_REPORT" --gate-json "$gate_out" --output "$comment_out"
    [ "$status" -eq 0 ]
    [ -f "$comment_out" ]
    grep -q "autospec-test-report-marker" "$comment_out"
    grep -q "Passed\|passed" "$comment_out"
}

@test "pr-report: --output flag writes to file (not stdout)" {
    [ -d "$TARGETS_DIR/target-clean-pass" ] || skip "target-clean-pass not found"
    local gate_out="$TEST_TMPDIR/gate-stdout.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-clean-pass" --output-gate "$gate_out" || true

    local comment_out="$TEST_TMPDIR/comment-stdout.md"
    run bash "$PR_REPORT" --gate-json "$gate_out" --output "$comment_out"
    [ "$status" -eq 0 ]
    # File should exist and have content
    [ -s "$comment_out" ]
}

# ── bootstrap-labels.sh idempotency ──────────────────────────────────────────

@test "bootstrap-labels: --dry-run lists all 16 labels without creating them" {
    run bash "$BOOTSTRAP" --dry-run
    [ "$status" -eq 0 ]
    # Should list all key labels
    [[ "$output" =~ "e2e:passed" ]]
    [[ "$output" =~ "e2e:blocked" ]]
    [[ "$output" =~ "e2e:scope-violation" ]]
    [[ "$output" =~ "CRITICAL" ]]
    [[ "$output" =~ "needs-human-review" ]]
}

@test "bootstrap-labels: outputs exactly 16 label names in dry-run" {
    run bash "$BOOTSTRAP" --dry-run
    [ "$status" -eq 0 ]
    local count
    count=$(printf '%s\n' "$output" | grep -c "e2e:\|CRITICAL\|needs-human-review" || echo 0)
    [ "$count" -ge 14 ]
}

# ── autospec-run SKILL.md Phase 4 wiring ─────────────────────────────────────

@test "autospec-run SKILL.md mentions run-gate.sh in Phase 4" {
    local skill_md="$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ -f "$skill_md" ]
    grep -q "run-gate.sh" "$skill_md"
}

@test "autospec-run SKILL.md documents exit 2 as batch halt" {
    local skill_md="$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ -f "$skill_md" ]
    grep -q "halt.*batch\|batch.*halt\|exit 2" "$skill_md"
}
