#!/usr/bin/env bats
# tests/unit/test_autospec_run_install_helpers.bats — autospec-run installer helper coverage.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    INSTALLER="$REPO_ROOT/skills/autospec-run/install.sh"
    TEST_TMP="$(mktemp -d)"
    export HOME="$TEST_TMP/home"
    export CODEX_HOME="$TEST_TMP/codex"
    mkdir -p "$HOME" "$CODEX_HOME"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "autospec-run installer leaves queue policy to the Rust control plane" {
    run bash "$INSTALLER" --harness codex

    [ "$status" -eq 0 ]
    for helper in heartbeat-write.sh heartbeat-read.sh autospec-usage-limit.sh; do
        [ -f "$HOME/.autospec/scripts/$helper" ]
        [ -x "$HOME/.autospec/scripts/$helper" ]
    done
    for removed in run-state.sh claim-issue.sh release-issue.sh issue-safety-gate.sh list-ready-issues.sh; do
        [ ! -e "$HOME/.autospec/scripts/$removed" ]
    done
}
