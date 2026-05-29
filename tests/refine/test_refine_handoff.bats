#!/usr/bin/env bats
# tests/refine/test_refine_handoff.bats — handoff modes (issue #672).
# Covers: --dry-run (no handoff), --interactive → /autospec, default → /autospec --autonomous.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"

setup() {
    TEST_TMP="$(mktemp -d -t refine-handoff.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    ART_DIR="$TEST_TMP/artifacts"
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"

    # PATH stubs for handoff target.
    STUB_BIN="$TEST_TMP/bin"
    mkdir -p "$STUB_BIN"
    HANDOFF_LOG="$TEST_TMP/handoff.log"

    # Stub 'claude' (used as the handoff dispatcher).
    cat > "$STUB_BIN/claude" <<EOF
#!/usr/bin/env bash
echo "claude-stub args=\$@" >> "$HANDOFF_LOG"
exit 0
EOF
    chmod +x "$STUB_BIN/claude"

    # Stub 'autospec' (used when invoked directly).
    cat > "$STUB_BIN/autospec" <<EOF
#!/usr/bin/env bash
echo "autospec-stub args=\$@" >> "$HANDOFF_LOG"
exit 0
EOF
    chmod +x "$STUB_BIN/autospec"

    export PATH="$STUB_BIN:$PATH"
    # Stubs live under macOS $TMPDIR; opt in to bypass the dispatcher
    # path-safety check (issue #681 Finding 6).
    export AUTOSPEC_HANDOFF_DISPATCHER=1
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

@test "--dry-run: writes artifact, exits 0, no handoff invocation" {
    run bash "$SCRIPT" "fix login button" --rounds 1 --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    f=$(ls "$ART_DIR"/*.json | head -1)
    [ -f "$f" ]
    run jq -r '.metadata.handoff_target' "$f"
    [ "$output" = "dry-run" ]
    run jq -r '.metadata.handoff_executed' "$f"
    [ "$output" = "false" ]
    [ ! -f "$HANDOFF_LOG" ]
}

@test "--interactive: invokes /autospec (interactive entry point) with final prompt" {
    run bash "$SCRIPT" "fix login button" --rounds 1 --interactive \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    [ -f "$HANDOFF_LOG" ]
    run cat "$HANDOFF_LOG"
    [[ "$output" == *"/autospec"* ]]
    [[ "$output" != *"--autonomous"* ]]
    f=$(ls "$ART_DIR"/*.json | head -1)
    run jq -r '.metadata.handoff_target' "$f"
    [ "$output" = "/autospec" ]
    run jq -r '.metadata.handoff_executed' "$f"
    [ "$output" = "true" ]
}

@test "default (autonomous): invokes /autospec --autonomous with final prompt" {
    run bash "$SCRIPT" "fix login button" --rounds 1 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    [ -f "$HANDOFF_LOG" ]
    run cat "$HANDOFF_LOG"
    [[ "$output" == *"/autospec"* ]]
    [[ "$output" == *"--autonomous"* ]]
    f=$(ls "$ART_DIR"/*.json | head -1)
    run jq -r '.metadata.handoff_target' "$f"
    [ "$output" = "/autospec --autonomous" ]
    run jq -r '.metadata.handoff_executed' "$f"
    [ "$output" = "true" ]
}
