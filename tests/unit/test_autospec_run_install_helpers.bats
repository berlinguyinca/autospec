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

@test "autospec-run installer installs distributed coordinator helpers" {
    run bash "$INSTALLER" --harness codex

    [ "$status" -eq 0 ]
    for helper in run-state.sh list-ready-issues.sh claim-issue.sh release-issue.sh heartbeat-write.sh heartbeat-read.sh autospec-usage-limit.sh; do
        [ -f "$HOME/.autospec/scripts/$helper" ]
        [ -x "$HOME/.autospec/scripts/$helper" ]
    done
}
