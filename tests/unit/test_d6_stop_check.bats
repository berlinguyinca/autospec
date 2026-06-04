#!/usr/bin/env bats
# tests/unit/test_d6_stop_check.bats — unit tests for scripts/autospec-stop-check.sh
#
# Spec ref: docs/specs/2026-06-04-skill-token-efficiency-design.md §5 D6
# Issue: #1025

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-stop-check.sh"
    export HOME="$(mktemp -d)"
    export FLAG_DIR="$HOME/.autospec"
    export FLAG_FILE="$FLAG_DIR/stop.flag"
    mkdir -p "$FLAG_DIR"
    # Provide a no-op autospec-stop.sh so the script can call it
    export AUTOSPEC_SCRIPTS_DIR="$HOME/scripts"
    mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
    cat > "$AUTOSPEC_SCRIPTS_DIR/autospec-stop.sh" <<'STUB'
#!/usr/bin/env bash
echo "[stub] autospec-stop.sh called with: $*"
exit 0
STUB
    chmod +x "$AUTOSPEC_SCRIPTS_DIR/autospec-stop.sh"
}

teardown() {
    rm -rf "$HOME"
}

# Positive: script is executable
@test "script is executable" {
    test -x "$SCRIPT"
}

# Positive: --help exits 0 and shows usage
@test "--help exits 0 and shows usage" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "autospec-stop-check.sh"
    echo "$output" | grep -q "Exit codes"
}

# Positive: no flag file → exit 0 (continue)
@test "no flag → exit 0 (continue)" {
    rm -f "$FLAG_FILE"
    run bash "$SCRIPT" "42" "feat/test" "step-push"
    [ "$status" -eq 0 ]
}

# Positive: flag with mode=graceful → exit 0 (inner-loop ignores graceful)
@test "graceful flag → exit 0 (inner loop does not abort)" {
    printf 'graceful\n2026-06-04T00:00:00Z user@host\n' > "$FLAG_FILE"
    run bash "$SCRIPT" "42" "feat/test" "step-push"
    [ "$status" -eq 0 ]
}

# Positive: flag with mode=immediate → exit 42 (abort signal)
@test "immediate flag → exit 42 (abort current issue)" {
    printf 'immediate\n2026-06-04T00:00:00Z user@host\n' > "$FLAG_FILE"
    run bash "$SCRIPT" "42" "feat/test" "step-push"
    [ "$status" -eq 42 ]
}

# Positive: immediate flag → calls autospec-stop.sh stub
@test "immediate flag → calls autospec-stop.sh with --abort-current-issue args" {
    printf 'immediate\n2026-06-04T00:00:00Z user@host\n' > "$FLAG_FILE"
    run bash "$SCRIPT" "99" "feat/branch" "step-pr"
    [ "$status" -eq 42 ]
    echo "$output" | grep -q "stub.*autospec-stop.sh"
}

# Negative: wrong arg count → exit 2 with error message
@test "wrong arg count → exit 2" {
    run bash "$SCRIPT" "only-one-arg"
    [ "$status" -eq 2 ]
    echo "$output" | grep -qi "error\|expected"
}

# Negative: no args → exit 2
@test "no args → exit 2" {
    run bash "$SCRIPT"
    [ "$status" -eq 2 ]
}

# Negative: caller idiom "if ! stop-check; then exit 0; fi" — immediate flag causes exit
@test "caller idiom: immediate flag causes the if-branch to trigger" {
    printf 'immediate\n2026-06-04T00:00:00Z user@host\n' > "$FLAG_FILE"
    # Simulate caller pattern: if ! stop-check; then echo ABORTED; fi
    run bash -c "
        if ! bash '$SCRIPT' '42' 'feat/test' 'step'; then
            echo ABORTED
        else
            echo CONTINUED
        fi
    "
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "ABORTED"
}

# Positive: no-flag caller idiom continues
@test "caller idiom: no flag → CONTINUED" {
    rm -f "$FLAG_FILE"
    run bash -c "
        if ! bash '$SCRIPT' '42' 'feat/test' 'step'; then
            echo ABORTED
        else
            echo CONTINUED
        fi
    "
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "CONTINUED"
}
