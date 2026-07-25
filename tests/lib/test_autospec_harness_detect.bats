#!/usr/bin/env bats
# tests/lib/test_autospec_harness_detect.bats — coverage for the harness-aware
# loop dispatcher resolver (issue #723).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    HELPER="$REPO_ROOT/scripts/lib/autospec-harness-detect.sh"
    [ -f "$HELPER" ] || skip "helper missing"
    PROBE_ROOT="$(mktemp -d -t autospec-harness-probe.XXXXXX)"
    BIN_DIR="$(mktemp -d -t autospec-harness-bin.XXXXXX)"
    SAFE_BIN_DIR="/usr/local/bin"
    # Reset env each test.
    unset AUTOSPEC_HANDOFF_DISPATCHER_KIND AUTOSPEC_HANDOFF_DISPATCHER
    unset AUTOSPEC_HARNESS_KIND AUTOSPEC_HARNESS_DISPATCHER
    unset CODEX_THREAD_ID CODEX_CI CLAUDECODE CLAUDE_CODE_ENTRYPOINT
    unset OPENCODE OPENCODE_SESSION_ID
    export AUTOSPEC_HARNESS_PROBE_ROOT="$PROBE_ROOT"
}

teardown() {
    rm -rf "$PROBE_ROOT" "$BIN_DIR" 2>/dev/null || true
}

# Helper: source the lib into a fresh subshell function call.
_source() {
    # shellcheck disable=SC1090
    unset _AUTOSPEC_HARNESS_DETECT_LOADED
    . "$HELPER"
}

@test "claude-code probe — ~/.claude/skills resolves to claude" {
    mkdir -p "$PROBE_ROOT/.claude/skills"
    _source
    run autospec_harness_detect
    [ "$status" -eq 0 ]
    [ "$output" = "claude" ]
}

@test "codex-cli probe — ~/.codex/prompts resolves to codex" {
    mkdir -p "$PROBE_ROOT/.codex/prompts"
    _source
    run autospec_harness_detect
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "opencode probe — ~/.config/opencode/agent resolves to opencode" {
    mkdir -p "$PROBE_ROOT/.config/opencode/agent"
    _source
    run autospec_harness_detect
    [ "$status" -eq 0 ]
    [ "$output" = "opencode" ]
}

@test "env override AUTOSPEC_HANDOFF_DISPATCHER_KIND wins over probe" {
    mkdir -p "$PROBE_ROOT/.claude/skills" "$PROBE_ROOT/.codex/prompts" \
        "$PROBE_ROOT/.config/opencode/agent"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=codex
    _source
    run autospec_harness_detect
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "Codex runtime marker wins over mixed installed harness homes" {
    mkdir -p "$PROBE_ROOT/.claude/skills" "$PROBE_ROOT/.codex/prompts" \
        "$PROBE_ROOT/.config/opencode/agent"
    export CODEX_THREAD_ID="test-thread"
    _source
    run autospec_harness_detect
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
    unset CODEX_THREAD_ID
}

@test "missing binary for detected harness exits 3 with named category" {
    # Probe says codex; PATH contains no codex binary.
    mkdir -p "$PROBE_ROOT/.codex/prompts"
    # Strip PATH down to a tmpdir with nothing in it.
    BASH_BIN="$(command -v bash)"
    run env -i HOME="$HOME" PATH="$BIN_DIR" \
        AUTOSPEC_HARNESS_PROBE_ROOT="$PROBE_ROOT" \
        "$BASH_BIN" -c ". '$HELPER'; autospec_harness_resolve_dispatcher"
    [ "$status" -eq 3 ]
    [[ "$output" == *"code_health:loop_handoff_no_dispatcher_for_harness"* ]]
    [[ "$output" == *"harness=codex"* ]]
}
