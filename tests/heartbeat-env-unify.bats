#!/usr/bin/env bats
# heartbeat-env-unify.bats — writers and the watchdog must resolve the SAME
# heartbeat dir whether the operator sets AUTOSPEC_HEARTBEAT_DIR or the
# back-compat alias AUTOSPEC_WATCHDOG_DIR. (Bug: they used different var names,
# so overriding one silently hid heartbeats from the other.)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    HBW="$REPO_ROOT/skills/autospec-run/scripts/heartbeat-write.sh"
    TMP="$(mktemp -d)"
    export AUTOSPEC_REPO="me/repo"
    unset AUTOSPEC_HEARTBEAT_DIR AUTOSPEC_WATCHDOG_DIR
}
teardown() { rm -rf "$TMP"; }

@test "heartbeat-write honors AUTOSPEC_WATCHDOG_DIR (back-compat alias)" {
    run env AUTOSPEC_WATCHDOG_DIR="$TMP/wd" bash "$HBW" --issue 1 --step claimed --repo me/repo
    [ "$status" -eq 0 ]
    [ -f "$TMP/wd/me_repo/1.json" ]
}

@test "heartbeat-write honors AUTOSPEC_HEARTBEAT_DIR" {
    run env AUTOSPEC_HEARTBEAT_DIR="$TMP/hb" bash "$HBW" --issue 2 --step claimed --repo me/repo
    [ "$status" -eq 0 ]
    [ -f "$TMP/hb/me_repo/2.json" ]
}

@test "AUTOSPEC_HEARTBEAT_DIR takes precedence when both are set (consistent across components)" {
    run env AUTOSPEC_HEARTBEAT_DIR="$TMP/hb" AUTOSPEC_WATCHDOG_DIR="$TMP/wd" bash "$HBW" --issue 3 --step claimed --repo me/repo
    [ "$status" -eq 0 ]
    [ -f "$TMP/hb/me_repo/3.json" ]
    [ ! -f "$TMP/wd/me_repo/3.json" ]
}

@test "all five heartbeat-dir resolvers use the unified both-var form" {
    # Guard against regressing one site back to a single-var resolution.
    for f in \
        "$REPO_ROOT/skills/autospec-run/scripts/heartbeat-write.sh" \
        "$REPO_ROOT/skills/autospec-run/scripts/heartbeat-read.sh" \
        "$REPO_ROOT/skills/autospec-run/scripts/autospec-run-status.sh" \
        "$REPO_ROOT/scripts/autospec-watchdog.sh" \
        "$REPO_ROOT/skills/autospec-shared/scripts/detect-monitor-exit-mode.sh"; do
        grep -qE 'AUTOSPEC_HEARTBEAT_DIR:-\$\{AUTOSPEC_WATCHDOG_DIR:-' "$f" \
            || { echo "MISSING unified resolver in $f"; false; }
    done
}
