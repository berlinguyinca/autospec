#!/usr/bin/env bats
# tests/disk-hygiene/test_round3_fixes.bats — regression tests for the round-3
# and round-4 review findings on the disk-hygiene PR. No fakes: real git, real
# worktrees, the real watchdog / disk-hygiene scripts, the real autospec adapter
# and the real gh (see gc_helpers.bash). Scratch lives under .test-tmp/.

load gc_helpers

setup() { gc_setup; }
teardown() { [ -z "${TEST_TMP:-}" ] || gc_teardown; }

# ── cleanup-worktree guards (round 3 #2, round 4 #9) ─────────────────────────
@test "cleanup_expect_head_keeps_a_worktree_whose_head_changed" {
    wt="$(make_worktree wt-a feat/a)"
    old="$(git -C "$wt" rev-parse HEAD)"
    commit_unpushed "$wt"
    run bash "$DH" cleanup-worktree --checked --expect-head "$old" "$wt"
    [ "$status" -eq 1 ]
    [ -d "$wt" ]
    [[ "$output" == *"head_changed"* ]]
}

@test "cleanup_expect_head_removes_when_head_unchanged" {
    wt="$(make_worktree wt-a feat/a)"
    run bash "$DH" cleanup-worktree --checked --expect-head "$(git -C "$wt" rev-parse HEAD)" "$wt"
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

@test "cleanup_empty_expect_head_fails_closed" {
    wt="$(make_worktree wt-a feat/a)"
    run bash "$DH" cleanup-worktree --expect-head "" "$wt"
    [ "$status" -eq 2 ]
    [ -d "$wt" ]
}

@test "cleanup_option_without_value_fails_instead_of_spinning" {
    wt="$(make_worktree wt-a feat/a)"
    for opt in --expect-head --reason; do
        run timeout 10 bash "$DH" cleanup-worktree "$opt"
        [ "$status" -eq 2 ]
    done
    [ -d "$wt" ]
}

@test "cleanup-enabled reflects the opt-out switches" {
    run bash "$DH" cleanup-enabled; [ "$status" -eq 0 ]
    AUTOSPEC_DISK_CLEANUP=off run bash "$DH" cleanup-enabled; [ "$status" -eq 1 ]
    AUTOSPEC_DISK_HYGIENE=off run bash "$DH" cleanup-enabled; [ "$status" -eq 1 ]
}

# ── main heartbeat loop: gh failure never deletes a live heartbeat (round 4 #1)
write_live_hb() {
    HB="$AUTOSPEC_WATCHDOG_DIR/berlinguyinca__autospec/900.json"
    printf '{"issue":"900","branch":"feat/900-x","step":"tests_started","ts":%s,"pr":"","repo":"%s","host":"%s"}\n' \
        "$(date -u +%s)" "$REAL_REPO" "$AUTOSPEC_HOST" > "$HB"
}

@test "main_loop_gh_failure_keeps_the_live_heartbeat (real gh, unreachable host)" {
    command -v gh >/dev/null || skip "gh not installed"
    unset AUTOSPEC_WATCHDOG_GC_ONLY
    write_live_hb
    GH_HOST=unreachable.invalid GH_TOKEN=x run bash "$WATCHDOG"
    [ "$status" -eq 0 ]
    [ -f "$HB" ]
    [[ "$output" == *"skipped=1"* ]]
}

@test "main_loop_hung_gh_is_bounded_and_keeps_the_heartbeat (real gh, blackholed host)" {
    command -v gh >/dev/null || skip "gh not installed"
    command -v timeout >/dev/null || command -v gtimeout >/dev/null || skip "no timeout binary"
    unset AUTOSPEC_WATCHDOG_GC_ONLY
    export AUTOSPEC_WATCHDOG_GC_GH_TIMEOUT_SECS=2
    write_live_hb
    start="$(date +%s)"
    GH_HOST=10.255.255.1 GH_TOKEN=x run bash "$WATCHDOG"
    [ $(( $(date +%s) - start )) -lt 60 ]
    [ -f "$HB" ]
}

# ── fallback and opt-out (round 3 #5/#6, round 4 #5/#7); dirty-guard and
# teardown-window coverage lives in test_round5_fixes.bats ───────────────────
@test "gc_fallback_without_hygiene_removes_a_clean_pushed_worktree" {
    require_real_gh; require_real_autospec
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    wt="$(make_worktree wt-21 "feat/$CLOSED_ISSUE-fb2")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
}

@test "gc_cleanup_off_still_reclaims_via_the_pre-existing guarded removal" {
    require_real_gh; require_real_autospec
    export AUTOSPEC_DISK_CLEANUP=off
    wt="$(make_worktree wt-30 "feat/$CLOSED_ISSUE-off")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
    [[ "$output" == *"garbage_collected=1"* ]]
}

@test "gc_number_slug_branch_is_issue_owned" {
    require_real_gh; require_real_autospec
    wt="$(make_worktree wt-s "feat/$CLOSED_ISSUE-released-heartbeat")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
}
