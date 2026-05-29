#!/usr/bin/env bats
# tests/continue/test_continue_rate_limit.bats — issue #700.
#
# Rate-limit + state-file invariants for autospec-continue.sh.
# Stubs claude on PATH so handoff succeeds, isolates HOME/PATH per test.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-continue.sh"
    WORK="$(mktemp -d -t autospec-continue-rl.XXXXXX)"
    STUB_BIN="$WORK/bin"
    mkdir -p "$STUB_BIN"

    cat > "$STUB_BIN/claude" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$STUB_BIN/claude"

    cat > "$STUB_BIN/refine-prompt.sh" <<'EOF'
#!/usr/bin/env bash
# Minimal stub: write extracted block to --output and exit 0.
out=""
prev=""
for a in "$@"; do
    if [ "$prev" = "--output" ]; then out="$a"; fi
    prev="$a"
done
printf 'REFINED: %s\n' "$1" > "$out"
EOF
    chmod +x "$STUB_BIN/refine-prompt.sh"

    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export PATH="$STUB_BIN:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"

    export AUTOSPEC_CONTINUE_HISTORY="$WORK/history.json"
    export HOME="$WORK"

    MSG="$WORK/msg.md"
    cat > "$MSG" <<'EOF'
Some prose.

## Next steps
- Ship the rate limiter.
- Add bats coverage.
EOF
}

teardown() {
    rm -rf "$WORK"
}

@test "duplicate source message within 60s exits 3 with continue_recent_duplicate" {
    export AUTOSPEC_CONTINUE_NOW=1000
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]

    # Second run at +10s (within 60s window) — must reject.
    export AUTOSPEC_CONTINUE_NOW=1010
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 3 ]
    [[ "$output" == *"code_health:continue_recent_duplicate"* ]]
}

@test "duplicate source message after 60s window is allowed" {
    export AUTOSPEC_CONTINUE_NOW=1000
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]

    export AUTOSPEC_CONTINUE_NOW=1100   # +100s, outside window
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
}

@test "oscillation: same extracted prompt 3x within 60min exits 3 with continue_oscillation" {
    # 3 distinct source messages whose extracted ## Next steps block is identical.
    common='## Next steps
- Same task repeated.
'
    M1="$WORK/m1.md"; M2="$WORK/m2.md"; M3="$WORK/m3.md"
    printf 'preamble A\n\n%s' "$common" > "$M1"
    printf 'preamble B\n\n%s' "$common" > "$M2"
    printf 'preamble C\n\n%s' "$common" > "$M3"

    export AUTOSPEC_CONTINUE_NOW=1000
    run bash "$SCRIPT" --from-message "$M1" --skip-refine
    [ "$status" -eq 0 ]
    export AUTOSPEC_CONTINUE_NOW=2000
    run bash "$SCRIPT" --from-message "$M2" --skip-refine
    [ "$status" -eq 0 ]
    # Third occurrence within 60min window of first — must trip oscillation.
    export AUTOSPEC_CONTINUE_NOW=3000
    run bash "$SCRIPT" --from-message "$M3" --skip-refine
    [ "$status" -eq 3 ]
    [[ "$output" == *"code_health:continue_oscillation"* ]]
}

@test "state file is deletable to reset rate limits" {
    export AUTOSPEC_CONTINUE_NOW=1000
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_CONTINUE_HISTORY" ]

    rm "$AUTOSPEC_CONTINUE_HISTORY"
    [ ! -f "$AUTOSPEC_CONTINUE_HISTORY" ]

    export AUTOSPEC_CONTINUE_NOW=1010  # Would have tripped duplicate, but state was reset.
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
}

@test "state file contains only hashes + timestamps, never prompt content" {
    export AUTOSPEC_CONTINUE_NOW=1000
    SECRET="distinctive_prompt_marker_xyz789"
    cat > "$MSG" <<EOF
some prose

## Next steps
- $SECRET
EOF
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_CONTINUE_HISTORY" ]

    # MUST NOT contain the prompt content.
    ! grep -q "$SECRET" "$AUTOSPEC_CONTINUE_HISTORY"
    ! grep -q "Next steps" "$AUTOSPEC_CONTINUE_HISTORY"

    # MUST contain the hash + timestamp schema fields.
    grep -q "source_message_hash" "$AUTOSPEC_CONTINUE_HISTORY"
    grep -q "extracted_prompt_hash" "$AUTOSPEC_CONTINUE_HISTORY"
    grep -q "timestamp" "$AUTOSPEC_CONTINUE_HISTORY"
}
