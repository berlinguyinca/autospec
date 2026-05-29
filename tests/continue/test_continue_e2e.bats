#!/usr/bin/env bats
# tests/continue/test_continue_e2e.bats — issue #700.
#
# End-to-end fixture: a conversation file whose last assistant message has
# a `## Next steps` section produces a refined-and-handoff pipeline (mocked).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-continue.sh"
    WORK="$(mktemp -d -t autospec-continue-e2e.XXXXXX)"
    STUB_BIN="$WORK/bin"
    mkdir -p "$STUB_BIN"
    STUB_LOG="$WORK/stub.log"
    export STUB_LOG

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

    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export PATH="$STUB_BIN:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"
    export AUTOSPEC_CONTINUE_HISTORY="$WORK/history.json"
    export HOME="$WORK"
}

teardown() {
    rm -rf "$WORK"
}

@test "e2e: conversation file -> extract -> refine -> handoff" {
    MSG="$WORK/conversation.md"
    cat > "$MSG" <<'EOF'
We landed the orchestrator and now want to add the rate limiter.

## Next steps
- Add SHA-256 hash bookkeeping to autospec-continue.sh.
- Wire validate.sh check_autospec_continue_contract.
- Cover with bats fixtures.
EOF

    export AUTOSPEC_CONTINUE_NOW=2000000
    # Use --skip-refine so we don't depend on a real refine pipeline.
    # The full path (extract -> handoff) is exercised; refine integration is
    # covered separately by test_continue_handoff.bats.
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine --autonomous
    [ "$status" -eq 0 ]

    # Dispatcher received the extracted block.
    [ -f "$STUB_LOG" ]
    grep -q 'STUB_CLAUDE' "$STUB_LOG"
    grep -q '/autospec' "$STUB_LOG"
    grep -q -- '--autonomous' "$STUB_LOG"

    # State file written with rate-limit schema.
    [ -f "$AUTOSPEC_CONTINUE_HISTORY" ]
    grep -q 'source_message_hash' "$AUTOSPEC_CONTINUE_HISTORY"
    grep -q 'extracted_prompt_hash' "$AUTOSPEC_CONTINUE_HISTORY"
}
