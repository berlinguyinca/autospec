#!/usr/bin/env bats
# tests/resume/test_watchdog_gc.bats — watchdog orphaned-worktree GC pass.
#
# A /tmp/wt-* worktree is pruned ONLY when it is clean, pushed, its issue is
# closed/unlabeled (or its PR provably ended the work) and no live heartbeat
# references it. Data-integrity contract: never remove un-pushed or dirty work,
# never remove a worktree with a fresh heartbeat.
#
# Everything is real: git repos and worktrees, the real watchdog in GC-only mode,
# the real autospec runtime-cleanup adapter, the real hostname and the real `gh`
# (read-only) against stable closed issue/PRs of berlinguyinca/autospec (see
# gc_helpers.bash). Only the tests that truly need GitHub's answer (an issue's
# state, a PR) call require_real_gh and skip without it; the data-integrity
# tests (unpushed, dirty, live heartbeat, non-issue branch) run on real local git
# alone and assert the guard's own log line, so they cannot pass vacuously.
# The PR decision logic is a pure function fed with gh's real output.

load ../disk-hygiene/gc_helpers

setup() { gc_setup; }
teardown() { [ -z "${TEST_TMP:-}" ] || gc_teardown; }

@test "gc_unpushed_not_pruned: a worktree with un-pushed commits is NOT removed" {
    wt="$(make_worktree wt-a "feat/$CLOSED_ISSUE-unpushed")"
    commit_unpushed "$wt"
    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" == *"keeping "*"wt-a (issue #$CLOSED_ISSUE): unpushed"* ]]
}

@test "gc_in_progress_not_pruned: a fresh heartbeat keeps the worktree" {
    wt="$(make_worktree wt-b "feat/$CLOSED_ISSUE-live")"
    printf '{"issue":"%s","branch":"feat/%s-live","step":"tests_started","ts":%s,"pr":"","repo":"%s","host":"%s"}\n' \
        "$CLOSED_ISSUE" "$CLOSED_ISSUE" "$(date -u +%s)" "$REAL_REPO" "$AUTOSPEC_HOST" \
        > "$AUTOSPEC_WATCHDOG_DIR/berlinguyinca__autospec/$CLOSED_ISSUE.json"
    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" != *"removed orphaned"* ]]
    [[ "$output" != *"keeping "* ]]   # stopped at the heartbeat guard, before any other check
}

@test "gc_clean_closed_pruned: clean + closed issue + no heartbeat is removed (real runtime cleanup)" {
    require_real_gh; require_real_autospec
    wt="$(make_worktree wt-c "feat/$CLOSED_ISSUE-clean")"
    real="$(cd "$wt" && pwd -P)"
    run_gc
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
    [[ "$output" == *"garbage_collected=1"* ]]
    run git -C "$PRIMARY" worktree list --porcelain
    [[ "$output" != *"$real"* ]]
}

@test "gc_runtime_cleanup_failure_preserves_the_worktree (real adapter failure)" {
    require_real_gh
    export AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"
    wt="$(make_worktree wt-d "feat/$CLOSED_ISSUE-rtfail")"
    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" == *"runtime cleanup failed"* ]]
}

@test "gc_dirty_worktree_keeps_runtime_resources (no teardown before the keep-check)" {
    export AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"   # a teardown attempt would log a failure
    wt="$(make_worktree wt-e "feat/$CLOSED_ISSUE-dirty")"
    echo dirty > "$wt/dirty.txt"
    run_gc
    [ -f "$wt/dirty.txt" ]
    [[ "$output" == *"keeping "*"wt-e (issue #$CLOSED_ISSUE): dirty"* ]]
    [[ "$output" != *"runtime cleanup failed"* ]]
}

@test "gc_unpushed_worktree_keeps_runtime_resources (no teardown before the keep-check)" {
    export AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"
    wt="$(make_worktree wt-f "feat/$CLOSED_ISSUE-unp2")"
    commit_unpushed "$wt"
    run_gc
    [ -d "$wt" ]
    [[ "$output" == *"keeping "*"wt-f (issue #$CLOSED_ISSUE): unpushed"* ]]
    [[ "$output" != *"runtime cleanup failed"* ]]
}

@test "gc_non_issue_branch_is_left_alone (explore-qa-gate-<PID>)" {
    export AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"
    wt="$(make_worktree wt-explore-qa-gate-12345 explore-qa-gate-12345)"
    ctl="$(make_worktree wt-ctl "feat/$CLOSED_ISSUE-ctl")"; commit_unpushed "$ctl"
    run_gc
    [ -d "$wt" ]
    [[ "$output" == *"keeping "*"wt-ctl "* ]]   # control: the candidate loop ran
    [[ "$output" != *"wt-explore-qa-gate"* ]]
    [[ "$output" != *"runtime cleanup failed"* ]]
}

# ── PR decision for an OPEN, active issue ────────────────────────────────────
# gh's real answers for real PRs feed the real production functions.

pr_line() { # branch -> the watchdog's real PR lookup against real GitHub
    require_real_gh
    . "$ROOT/scripts/autospec-timeout.sh"
    load_watchdog_fn _gc_pr_line
    REPO_ARGS="--repo $REAL_REPO" WATCHDOG_GC_GH_TIMEOUT_SECS=60 _gc_pr_line "$PWD" "$1"
}

@test "real gh: a MERGED PR is reported with its head oid" {
    [ "$(pr_line "$MERGED_PR_BRANCH")" = "MERGED $MERGED_PR_OID" ]
}

@test "real gh: a CLOSED PR is reported with its head oid" {
    [ "$(pr_line "$CLOSED_PR_BRANCH")" = "CLOSED $CLOSED_PR_OID" ]
}

@test "real gh: a branch without a PR yields no PR line" {
    [ -z "$(tr -d ' ' <<<"$(pr_line "no-such-branch-$$")")" ]
}

@test "decision: MERGED at the worktree HEAD releases an active issue" {
    load_watchdog_fn _gc_pr_decision
    WATCHDOG_GC_PR_CLOSED_GRACE_SECS=3600
    _gc_pr_decision "$(pr_line "$MERGED_PR_BRANCH")" "$MERGED_PR_OID" 0
}

@test "decision: CLOSED at HEAD is kept within the grace period" {
    load_watchdog_fn _gc_pr_decision
    WATCHDOG_GC_PR_CLOSED_GRACE_SECS=3600
    ! _gc_pr_decision "$(pr_line "$CLOSED_PR_BRANCH")" "$CLOSED_PR_OID" 10
}

@test "decision: CLOSED at HEAD is released after the grace period" {
    load_watchdog_fn _gc_pr_decision
    WATCHDOG_GC_PR_CLOSED_GRACE_SECS=3600
    _gc_pr_decision "$(pr_line "$CLOSED_PR_BRANCH")" "$CLOSED_PR_OID" 7200
}

@test "decision: a PR for a different HEAD keeps the worktree" {
    load_watchdog_fn _gc_pr_decision
    WATCHDOG_GC_PR_CLOSED_GRACE_SECS=0
    ! _gc_pr_decision "$(pr_line "$CLOSED_PR_BRANCH")" "0000000000000000000000000000000000000000" 9999
}

@test "decision: no PR keeps the worktree" {
    load_watchdog_fn _gc_pr_decision
    WATCHDOG_GC_PR_CLOSED_GRACE_SECS=0
    ! _gc_pr_decision "$(pr_line "no-such-branch-$$")" "$MERGED_PR_OID" 9999
}
