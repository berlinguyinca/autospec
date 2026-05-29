#!/usr/bin/env bats
# tests/continue/test_continue_handoff.bats — autospec-continue orchestrator (#699).
#
# Stubs `claude` and `autospec` on PATH so we can assert which dispatcher
# the orchestrator routes to and with what args. Each test isolates HOME
# and PATH so we do not touch the operator's real env.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-continue.sh"
    WORK="$(mktemp -d -t autospec-continue.XXXXXX)"
    STUB_BIN="$WORK/bin"
    mkdir -p "$STUB_BIN"
    # Capture log written by stubs.
    STUB_LOG="$WORK/stub.log"
    export STUB_LOG

    # Stub `autospec` — logs argv to STUB_LOG, exits 0.
    cat > "$STUB_BIN/autospec" <<'EOF'
#!/usr/bin/env bash
{
    printf 'STUB_AUTOSPEC'
    for a in "$@"; do printf '|%s' "$a"; done
    printf '\n'
} >> "$STUB_LOG"
exit 0
EOF
    chmod +x "$STUB_BIN/autospec"

    # Stub `claude` — logs argv to STUB_LOG, exits 0.
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

    # Isolate rate-limit state per test (issue #700).
    export HOME="$WORK"
    export AUTOSPEC_CONTINUE_HISTORY="$WORK/continue-history.json"

    # AUTOSPEC_HANDOFF_DISPATCHER=1 needed because stubs live in tmpdir.
    export AUTOSPEC_HANDOFF_DISPATCHER=1
    # Prepend stub dir so it wins over real binaries (if any) on PATH.
    # Keep core utils available.
    export PATH="$STUB_BIN:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"

    # Sample source message with a clear ## Next steps section.
    MSG="$WORK/msg.md"
    cat > "$MSG" <<'EOF'
We just landed the extraction helper.

## Next steps
- Ship the orchestrator that wires extraction into refine and handoff.
- Add bats coverage for the four flags.
EOF
}

teardown() {
    rm -rf "$WORK"
}

@test "--skip-refine routes directly to autospec without going through refine" {
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    # Stub got invoked.
    [ -f "$STUB_LOG" ]
    grep -q '^STUB_CLAUDE\|^STUB_AUTOSPEC' "$STUB_LOG"
    # Refine artifact directory should NOT exist (we bypassed refine).
    [ ! -d "$WORK/.autospec/refinements" ]
}

@test "default route invokes refine which then hands off via stub" {
    # Refine writes its artifact under --artifact-dir; we redirect to WORK.
    run bash "$SCRIPT" --from-message "$MSG" \
        --artifact-dir "$WORK/refinements" \
        --repo-root "$REPO_ROOT"
    [ "$status" -eq 0 ]
    # Refine left behind a JSON artifact.
    ls "$WORK/refinements"/*.json >/dev/null
    # Handoff dispatcher was invoked (claude wins over autospec on PATH).
    grep -q '^STUB_CLAUDE\|^STUB_AUTOSPEC' "$STUB_LOG"
}

@test "--ask-confirm with operator 'proceed' completes handoff" {
    run bash -c "printf 'proceed\n' | '$SCRIPT' --from-message '$MSG' --skip-refine --ask-confirm"
    [ "$status" -eq 0 ]
    grep -q '^STUB_CLAUDE\|^STUB_AUTOSPEC' "$STUB_LOG"
}

@test "--ask-confirm with operator 'cancel' aborts before handoff" {
    run bash -c "printf 'cancel\n' | '$SCRIPT' --from-message '$MSG' --skip-refine --ask-confirm"
    # Cancel exits non-zero (3) and no stub call recorded.
    [ "$status" -eq 3 ]
    [ ! -s "$STUB_LOG" ] || ! grep -q '^STUB_' "$STUB_LOG"
}

@test "--from-message reads from the file path end-to-end" {
    # Confirms the file-source path works without any extra plumbing.
    run bash "$SCRIPT" --from-message "$MSG" --skip-refine
    [ "$status" -eq 0 ]
    # The extracted recommendation must have been forwarded to the dispatcher.
    # Stubs log the full argv joined by '|'; assert the keyword "orchestrator"
    # (from the ## Next steps bullet) reaches the dispatcher.
    grep -qi 'orchestrator' "$STUB_LOG"
}

@test "--from-message rejects a missing file with exit 2" {
    run bash "$SCRIPT" --from-message "$WORK/does-not-exist.md" --skip-refine
    [ "$status" -eq 2 ]
}
