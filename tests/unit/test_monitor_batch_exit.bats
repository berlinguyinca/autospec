#!/usr/bin/env bats
# tests/unit/test_monitor_batch_exit.bats — verify check_monitor_batch_exit()
# in scripts/validate.sh detects missing/present batch self-termination logic.
#
# Five cases per spec docs/specs/2026-05-07-monitor-session-reset-design.md:
#   1. SKILL.md with all required batch-exit tokens passes
#   2. SKILL.md missing batch_issue_count fails
#   3. SKILL.md missing batch-done.json token fails
#   4. SKILL.md without Phase 4 marker is silently skipped (exit 0)
#   5. SKILL.md missing ALL_DONE fails

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    SCRATCH="$(mktemp -d)"
    export SCRATCH REPO_ROOT VALIDATE

    # Build an isolated helper that exposes only check_monitor_batch_exit.
    HELPER="$SCRATCH/helper.sh"
    cat > "$HELPER" <<'HELPER_SCRIPT'
#!/usr/bin/env bash
set -eu
fail() { printf 'validate: FAIL — %s\n' "$*" >&2; exit 1; }
info() { printf 'validate: %s\n' "$*"; }
HELPER_SCRIPT

    sed -n '/^check_monitor_batch_exit()/,/^}/p' "$VALIDATE" >> "$HELPER"
    export HELPER
}

teardown() {
    if [ -n "${SCRATCH:-}" ] && [ -d "$SCRATCH" ]; then
        rm -rf "$SCRATCH"
    fi
}

# Helper: write a SKILL.md body with Phase 4 marker and all required tokens.
_full_batch_skill() {
    local f="$1"
    cat > "$f" <<'EOF'
---
name: autospec-run
version: 1.0.0
---

## Phase 4 — Background autonomous monitor

> batch_issue_count=0; AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-3}
>
> Write "$HOME/.autospec/batch-done.json" with status BATCH_COMPLETE after batch limit.
> When queue drained write status ALL_DONE instead.
EOF
}

# ===========================================================================
# Test 1: All tokens present → passes
# ===========================================================================
@test "check_monitor_batch_exit: SKILL.md with all batch-exit tokens passes" {
    local f="$SCRATCH/SKILL.md"
    _full_batch_skill "$f"

    run bash -c "source '$HELPER' && check_monitor_batch_exit '$f'" 2>&1
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "monitor-batch-exit"
}

# ===========================================================================
# Test 2: Missing batch_issue_count → fails
# ===========================================================================
@test "check_monitor_batch_exit: SKILL.md missing batch_issue_count fails" {
    local f="$SCRATCH/SKILL.md"
    _full_batch_skill "$f"
    # Remove the batch_issue_count line
    sed -i '' '/batch_issue_count/d' "$f" 2>/dev/null \
        || sed -i '/batch_issue_count/d' "$f"

    run bash -c "source '$HELPER' && check_monitor_batch_exit '$f'" 2>&1
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "batch_issue_count"
}

# ===========================================================================
# Test 3: Missing batch-done.json → fails
# ===========================================================================
@test "check_monitor_batch_exit: SKILL.md missing batch-done.json fails" {
    local f="$SCRATCH/SKILL.md"
    _full_batch_skill "$f"
    sed -i '' '/batch-done\.json/d' "$f" 2>/dev/null \
        || sed -i '/batch-done\.json/d' "$f"

    run bash -c "source '$HELPER' && check_monitor_batch_exit '$f'" 2>&1
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "batch-done.json"
}

# ===========================================================================
# Test 4: No Phase 4 marker → silently skipped (exit 0)
# ===========================================================================
@test "check_monitor_batch_exit: SKILL.md without Phase 4 is silently skipped" {
    local f="$SCRATCH/SKILL.md"
    cat > "$f" <<'EOF'
---
name: autospec-classify
version: 1.0.0
---

# autospec-classify

This skill has no background autonomous monitor outer loop.
EOF

    run bash -c "source '$HELPER' && check_monitor_batch_exit '$f'" 2>&1
    [ "$status" -eq 0 ]
    # Must NOT print the monitor-batch-exit info line (skipped silently)
    ! echo "$output" | grep -q "monitor-batch-exit"
}

# ===========================================================================
# Test 5: Missing ALL_DONE → fails
# ===========================================================================
@test "check_monitor_batch_exit: SKILL.md missing ALL_DONE fails" {
    local f="$SCRATCH/SKILL.md"
    _full_batch_skill "$f"
    sed -i '' '/ALL_DONE/d' "$f" 2>/dev/null \
        || sed -i '/ALL_DONE/d' "$f"

    run bash -c "source '$HELPER' && check_monitor_batch_exit '$f'" 2>&1
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "ALL_DONE"
}
