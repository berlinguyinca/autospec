#!/usr/bin/env bats
# tests/continue/test_continue_loop.bats — issue #710.
#
# Default-on continuous loop behavior for /autospec-continue. The default
# invocation runs the shared loop driver (scripts/lib/autospec-loop.sh, #708)
# until convergence; --no-loop / --once and ~/.autospec/continue-no-loop.flag
# bypass the loop and restore today's single-pass behavior.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-continue.sh"
    WORK="$(mktemp -d -t autospec-continue-loop.XXXXXX)"
    STUB_BIN="$WORK/bin"
    mkdir -p "$STUB_BIN"
    STUB_LOG="$WORK/stub.log"
    export STUB_LOG

    # `claude` stub — logs argv (one line per call), exits 0.
    cat > "$STUB_BIN/claude" <<'EOF'
#!/usr/bin/env bash
{
    printf 'STUB_CLAUDE'
    for a in "$@"; do printf '|%s' "$a"; done
    printf '\n'
} >> "$STUB_LOG"
exit 0
EOF
    chmod +x "$STUB_BIN/claude"

    export HOME="$WORK"
    export AUTOSPEC_CONTINUE_HISTORY="$WORK/continue-history.json"
    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude
    export PATH="$STUB_BIN:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"

    MSG="$WORK/msg.md"
    cat > "$MSG" <<'EOF'
Some prose.

## Next steps
- Ship the loop default-on change.
EOF
}

teardown() {
    rm -rf "$WORK"
}

@test "--no-loop runs exactly once (single-pass legacy behavior)" {
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine --no-loop
    [ "$status" -eq 0 ]
    # Exactly one handoff invocation.
    n=$(grep -c '^STUB_CLAUDE' "$STUB_LOG" 2>/dev/null || echo 0)
    [ "$n" -eq 1 ]
}

@test "--once is an alias for --no-loop" {
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine --once
    [ "$status" -eq 0 ]
    n=$(grep -c '^STUB_CLAUDE' "$STUB_LOG" 2>/dev/null || echo 0)
    [ "$n" -eq 1 ]
}

@test "~/.autospec/continue-no-loop.flag forces single-pass" {
    mkdir -p "$HOME/.autospec"
    : > "$HOME/.autospec/continue-no-loop.flag"
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    n=$(grep -c '^STUB_CLAUDE' "$STUB_LOG" 2>/dev/null || echo 0)
    [ "$n" -eq 1 ]
}

@test "default invocation enters loop mode (announces loop on stderr)" {
    # Force the loop to converge on iteration 1 by setting MAX_ITERATIONS=1
    # via the env var the shared driver consumes. The dispatcher stub returns
    # 0 immediately, so the loop terminates at the round cap after 1 pass.
    export AUTOSPEC_LOOP_MAX_ITERATIONS=1
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    # The loop announces itself on stderr.
    [[ "$output" == *"loop"* ]] || [[ "$output" == *"continuous"* ]] || [[ "$output" == *"iteration"* ]]
}

@test "--help mentions --no-loop opt-out" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--no-loop"* ]]
}
