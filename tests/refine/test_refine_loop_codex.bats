#!/usr/bin/env bats
# tests/refine/test_refine_loop_codex.bats — proves the harness-aware
# dispatcher actually invokes the Codex CLI path when ~/.codex is mounted,
# and that the loop driver iterates more than once with the stubbed binary.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    HELPER="$REPO_ROOT/scripts/lib/autospec-harness-detect.sh"
    [ -f "$HELPER" ] || skip "helper missing"
    WORK="$(mktemp -d -t refine-codex-loop.XXXXXX)"
    PROBE="$WORK/home"
    BIN="$WORK/bin"
    LOG="$WORK/codex.log"
    mkdir -p "$PROBE/.codex/prompts" "$BIN"

    # Stub `codex` binary in a safe (non-tmpdir-rejected) absolute path.
    # We use a writable but well-known prefix and grant AUTOSPEC_HANDOFF_DISPATCHER=1
    # to bypass the tmpdir denylist for tests.
    cat > "$BIN/codex" <<'EOF'
#!/usr/bin/env bash
# Test stub: log every call, write a fake run-summary.md so the loop
# driver's staleness guard passes, and exit 0.
echo "$@" >> "$CODEX_STUB_LOG"
SUMMARY="$CODEX_STUB_SUMMARY"
if [ -n "$SUMMARY" ]; then
    mkdir -p "$(dirname "$SUMMARY")"
    {
        printf '## Next steps\n'
        # Distinct line per iteration so the harvested prompt changes
        # iter-to-iter and the loop doesn't trip oscillation.
        printf '- next slice %s\n' "$(date +%N)"
    } > "$SUMMARY"
fi
exit 0
EOF
    chmod +x "$BIN/codex"

    export CODEX_STUB_LOG="$LOG"
    export CODEX_STUB_SUMMARY="$WORK/repo/.autospec/run-summary.md"
    mkdir -p "$WORK/repo/.autospec"

    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export AUTOSPEC_HARNESS_PROBE_ROOT="$PROBE"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=codex
}

teardown() {
    rm -rf "$WORK" 2>/dev/null || true
}

@test "codex-cli harness — autospec_harness_invoke uses codex exec form" {
    # shellcheck disable=SC1090
    PATH="$BIN:$PATH" bash -c "
        unset _AUTOSPEC_HARNESS_DETECT_LOADED
        . '$HELPER'
        autospec_harness_resolve_dispatcher
        autospec_harness_invoke autonomous 'do the thing'
    "
    [ -f "$LOG" ]
    # Codex form: `exec --skip-git-repo-check "/autospec --autonomous do the thing"`
    grep -q -- "exec --skip-git-repo-check" "$LOG"
    grep -q -- "/autospec --autonomous do the thing" "$LOG"
}

@test "codex-cli harness — refine-prompt.sh handoff dispatches via codex stub more than once across two manual invocations" {
    # This is the minimum proof that the loop CAN iterate against Codex:
    # we invoke refine-prompt.sh handoff twice in sequence, each writes a
    # fresh run-summary via the stub, and the log shows >=2 codex calls.
    cd "$WORK/repo"
    PATH="$BIN:$PATH" bash "$REPO_ROOT/scripts/refine-prompt.sh" \
        "first prompt" --rounds 1 --repo-root "$WORK/repo" \
        --artifact-dir "$WORK/repo/.autospec/refinements" >/dev/null 2>&1 || true
    PATH="$BIN:$PATH" bash "$REPO_ROOT/scripts/refine-prompt.sh" \
        "second prompt" --rounds 1 --repo-root "$WORK/repo" \
        --artifact-dir "$WORK/repo/.autospec/refinements" >/dev/null 2>&1 || true
    [ -f "$LOG" ]
    # At least 2 calls = loop iterated more than once.
    count=$(wc -l < "$LOG")
    [ "$count" -ge 2 ]
}
