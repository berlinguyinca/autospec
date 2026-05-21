#!/usr/bin/env bats
# skills/autospec-test/tests/integration/run-against-target.bats
#
# Integration harness: runs run-gate.sh against each synthetic target and
# diffs actual gate JSON against checked-in goldens.
#
# Usage:
#   bats skills/autospec-test/tests/integration/run-against-target.bats
#
# To run a single target:
#   bats skills/autospec-test/tests/integration/run-against-target.bats --filter "target-clean-pass"

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    TARGETS_DIR="$REPO_ROOT/skills/autospec-test/test-targets"
    GOLDEN_DIR="$REPO_ROOT/skills/autospec-test/tests/integration/golden"
    RUN_GATE="$SCRIPTS_DIR/run-gate.sh"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-integration-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── Helper: normalize gate JSON for stable diff ───────────────────────────────
# Removes timing fields and sorts keys so diff is deterministic.
normalize_gate_json() {
    local json="$1"
    printf '%s' "$json" | jq --sort-keys 'del(.ts, .elapsed_ms, .started_at, .ended_at)'
}

# ── target-clean-pass ─────────────────────────────────────────────────────────

@test "integration: target-clean-pass gate passes (overall_passed=true)" {
    [ -d "$TARGETS_DIR/target-clean-pass" ] || skip "target-clean-pass not found"
    local gate_out="$TEST_TMPDIR/gate-clean-pass.json"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-clean-pass" --output-gate "$gate_out"
    [ "$status" -eq 0 ]
    [ -f "$gate_out" ]
    local passed
    passed=$(jq -r '.overall_passed' "$gate_out")
    [ "$passed" = "true" ]
}

@test "integration: target-clean-pass gate JSON matches golden" {
    [ -d "$TARGETS_DIR/target-clean-pass" ] || skip "target-clean-pass not found"
    local gate_out="$TEST_TMPDIR/gate-clean-pass.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-clean-pass" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]
    local actual golden
    actual=$(normalize_gate_json "$(cat "$gate_out")")
    golden=$(normalize_gate_json "$(cat "$GOLDEN_DIR/target-clean-pass/gate.json")")
    [ "$actual" = "$golden" ]
}

# ── target-failing-gap ────────────────────────────────────────────────────────

@test "integration: target-failing-gap gate fails (overall_passed=false)" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    local gate_out="$TEST_TMPDIR/gate-failing-gap.json"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap" --output-gate "$gate_out"
    [ "$status" -ne 0 ]
    [ -f "$gate_out" ]
    local passed
    passed=$(jq -r '.overall_passed' "$gate_out")
    [ "$passed" = "false" ]
}

@test "integration: target-failing-gap gate shows missing drag_drop + missing UI element" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    local gate_out="$TEST_TMPDIR/gate-failing-gap.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]
    # Check for drag_drop in missing_behavior_categories or reason
    local content
    content=$(cat "$gate_out")
    echo "$content" | grep -q "drag_drop" || {
        echo "Expected drag_drop in gate output, got: $content"
        false
    }
    echo "$content" | grep -q "drag-handle\|drag_drop\|missing_ui" || {
        echo "Expected missing UI element in gate output"
        false
    }
}

@test "integration: target-failing-gap gate JSON matches golden" {
    [ -d "$TARGETS_DIR/target-failing-gap" ] || skip "target-failing-gap not found"
    local gate_out="$TEST_TMPDIR/gate-failing-gap.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-failing-gap" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]
    local actual golden
    actual=$(normalize_gate_json "$(cat "$gate_out")")
    golden=$(normalize_gate_json "$(cat "$GOLDEN_DIR/target-failing-gap/gate.json")")
    [ "$actual" = "$golden" ]
}

# ── target-greenwash-bait ─────────────────────────────────────────────────────

@test "integration: target-greenwash-bait gate fails with unjustified-shift reason" {
    [ -d "$TARGETS_DIR/target-greenwash-bait" ] || skip "target-greenwash-bait not found"
    local gate_out="$TEST_TMPDIR/gate-greenwash.json"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-greenwash-bait" --output-gate "$gate_out"
    [ "$status" -ne 0 ]
    [ -f "$gate_out" ]
    local content
    content=$(cat "$gate_out")
    echo "$content" | grep -q "unjustified-shift\|LOOSENING" || {
        echo "Expected unjustified-shift in gate output, got: $content"
        false
    }
}

@test "integration: target-greenwash-bait gate JSON matches golden" {
    [ -d "$TARGETS_DIR/target-greenwash-bait" ] || skip "target-greenwash-bait not found"
    local gate_out="$TEST_TMPDIR/gate-greenwash.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-greenwash-bait" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]
    local actual golden
    actual=$(normalize_gate_json "$(cat "$gate_out")")
    golden=$(normalize_gate_json "$(cat "$GOLDEN_DIR/target-greenwash-bait/gate.json")")
    [ "$actual" = "$golden" ]
}

# ── target-mode-ii-fixture ────────────────────────────────────────────────────

@test "integration: target-mode-ii-fixture gate fails with scope-violation" {
    [ -d "$TARGETS_DIR/target-mode-ii-fixture" ] || skip "target-mode-ii-fixture not found"
    local gate_out="$TEST_TMPDIR/gate-mode-ii.json"
    run bash "$RUN_GATE" "$TARGETS_DIR/target-mode-ii-fixture" --output-gate "$gate_out"
    [ "$status" -ne 0 ]
    [ -f "$gate_out" ]
    local content
    content=$(cat "$gate_out")
    echo "$content" | grep -q "scope.violation\|scope_violation" || {
        echo "Expected scope-violation in gate output, got: $content"
        false
    }
    echo "$content" | grep -q "restore_invoked\|restore.invoked" || {
        echo "Expected restore_invoked in gate output"
        false
    }
}

@test "integration: target-mode-ii-fixture gate JSON matches golden" {
    [ -d "$TARGETS_DIR/target-mode-ii-fixture" ] || skip "target-mode-ii-fixture not found"
    local gate_out="$TEST_TMPDIR/gate-mode-ii.json"
    bash "$RUN_GATE" "$TARGETS_DIR/target-mode-ii-fixture" --output-gate "$gate_out" || true
    [ -f "$gate_out" ]
    local actual golden
    actual=$(normalize_gate_json "$(cat "$gate_out")")
    golden=$(normalize_gate_json "$(cat "$GOLDEN_DIR/target-mode-ii-fixture/gate.json")")
    [ "$actual" = "$golden" ]
}

# ── Language matrix targets ───────────────────────────────────────────────────

@test "integration: lang-matrix/node target exists and has test file" {
    [ -d "$TARGETS_DIR/lang-matrix/node" ] || skip "lang-matrix/node not found"
    [ -f "$TARGETS_DIR/lang-matrix/node/package.json" ]
    # Verify a test file exists
    local test_count
    test_count=$(find "$TARGETS_DIR/lang-matrix/node" -name "*.test.*" -o -name "*.spec.*" 2>/dev/null | wc -l | tr -d ' ')
    [ "$test_count" -gt 0 ]
}

@test "integration: lang-matrix/python target exists and has test file" {
    [ -d "$TARGETS_DIR/lang-matrix/python" ] || skip "lang-matrix/python not found"
    local test_count
    test_count=$(find "$TARGETS_DIR/lang-matrix/python" -name "test_*.py" -o -name "*_test.py" 2>/dev/null | wc -l | tr -d ' ')
    [ "$test_count" -gt 0 ]
}

@test "integration: lang-matrix/go target exists and has test file" {
    [ -d "$TARGETS_DIR/lang-matrix/go" ] || skip "lang-matrix/go not found"
    local test_count
    test_count=$(find "$TARGETS_DIR/lang-matrix/go" -name "*_test.go" 2>/dev/null | wc -l | tr -d ' ')
    [ "$test_count" -gt 0 ]
}

@test "integration: lang-matrix/rust target exists and has test" {
    [ -d "$TARGETS_DIR/lang-matrix/rust" ] || skip "lang-matrix/rust not found"
    [ -f "$TARGETS_DIR/lang-matrix/rust/Cargo.toml" ]
    # Rust tests are inline; check for #[test] attribute
    local test_count
    test_count=$(grep -r "#\[test\]" "$TARGETS_DIR/lang-matrix/rust/src/" 2>/dev/null | wc -l | tr -d ' ')
    [ "$test_count" -gt 0 ]
}

@test "integration: lang-matrix/jvm target exists and has test file" {
    [ -d "$TARGETS_DIR/lang-matrix/jvm" ] || skip "lang-matrix/jvm not found"
    local test_count
    test_count=$(find "$TARGETS_DIR/lang-matrix/jvm" -name "*Test.java" -o -name "*Tests.java" 2>/dev/null | wc -l | tr -d ' ')
    [ "$test_count" -gt 0 ]
}
