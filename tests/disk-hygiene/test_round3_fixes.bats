#!/usr/bin/env bats
# tests/disk-hygiene/test_round3_fixes.bats — regression tests for the round-3
# and round-4 review findings on the disk-hygiene PR. No fakes: real git, real
# worktrees, the real watchdog / disk-hygiene scripts, the real autospec adapter
# and the real gh (see gc_helpers.bash). Scratch lives under .test-tmp/.

load gc_helpers

setup() { gc_setup; }
teardown() { [ -z "${TEST_TMP:-}" ] || gc_teardown; }

# ── ownership: other repos' worktrees (round 3 #1, round 4 #8) ───────────────
@test "gc_other_repos_worktree_is_left_alone (colliding issue number) and says why" {
    require_real_gh; require_real_autospec
    mkdir -p "$TEST_TMP/acme"
    git init -q --bare "$TEST_TMP/acme/other.git"
    git clone -q "$TEST_TMP/acme/other.git" "$TEST_TMP/other-primary" 2>/dev/null
    git -C "$TEST_TMP/other-primary" checkout -q -b main
    echo s > "$TEST_TMP/other-primary/s.txt"; git -C "$TEST_TMP/other-primary" add -A
    git -C "$TEST_TMP/other-primary" commit -q -m s
    git -C "$TEST_TMP/other-primary" push -q -u origin main
    other="$(REPO_FOR_WT="$TEST_TMP/other-primary" make_worktree wt-x "feat/autospec-issue-$CLOSED_ISSUE" origin/main)"
    mine="$(make_worktree wt-y "feat/$CLOSED_ISSUE-mine")"
    run bash "$WATCHDOG"
    [ "$status" -eq 0 ]
    [ -d "$other" ]
    [ ! -e "$mine" ]
    [[ "$output" == *"foreign repo"* ]]
}

@test "gc_fork_origin_checkout_with_upstream_remote_is_owned" {
    require_real_gh; require_real_autospec
    mkdir -p "$TEST_TMP/me"
    git clone -q --bare "$ORIGIN" "$TEST_TMP/me/fork.git"
    git -C "$PRIMARY" remote add upstream "$ORIGIN"          # URL ends in berlinguyinca/autospec.git
    git -C "$PRIMARY" remote set-url origin "$TEST_TMP/me/fork.git"   # origin is a fork
    git -C "$PRIMARY" fetch -q origin
    wt="$(make_worktree wt-z "feat/$CLOSED_ISSUE-fork")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
}

# ── issue ownership by branch name (round 3 #8, round 4 #4) ──────────────────
@test "branch -> issue: only real issue branches parse" {
    load_watchdog_fn _gc_issue_of_branch
    [ "$(_gc_issue_of_branch feat/autospec-issue-700)" = 700 ]
    [ "$(_gc_issue_of_branch fix/issue-12)" = 12 ]
    [ "$(_gc_issue_of_branch feat/2879-released-heartbeat)" = 2879 ]
    [ -z "$(_gc_issue_of_branch release/2026-11-18)" ]
    [ -z "$(_gc_issue_of_branch backup/20261118)" ]
    [ -z "$(_gc_issue_of_branch hotfix/1)" ]
    [ -z "$(_gc_issue_of_branch feat/123456789012-too-long)" ]
    [ -z "$(_gc_issue_of_branch explore-qa-gate-12345)" ]
    [ -z "$(_gc_issue_of_branch codex/foo)" ]
}

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

# ── dh_fetch ssh handling (round 3 #4, round 4 #2/#3) ────────────────────────
ssh_wrapper() { # logs its args, then runs the remote command locally
    cat > "$TEST_TMP/myssh" <<EOS
#!/usr/bin/env bash
echo "\$*" >> "$TEST_TMP/ssh.log"
last=""; for a in "\$@"; do last="\$a"; done
exec sh -c "\$last"
EOS
    chmod +x "$TEST_TMP/myssh"
    git -C "$PRIMARY" remote set-url origin "ssh://nohost$ORIGIN"
}

@test "dh_fetch_respects_core_sshCommand and adds BatchMode" {
    ssh_wrapper
    git -C "$PRIMARY" config core.sshCommand "$TEST_TMP/myssh -i key"
    bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"
    grep -q -- "-i key" "$TEST_TMP/ssh.log"
    grep -q "BatchMode=yes" "$TEST_TMP/ssh.log"
}

@test "dh_fetch_appends_BatchMode_to_GIT_SSH_COMMAND" {
    ssh_wrapper
    GIT_SSH_COMMAND="$TEST_TMP/myssh -i key2" bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"
    grep -q -- "-i key2" "$TEST_TMP/ssh.log"
    grep -q "BatchMode=yes" "$TEST_TMP/ssh.log"
}

@test "dh_fetch_does_not_override_GIT_SSH" {
    ssh_wrapper
    GIT_SSH="$TEST_TMP/myssh" bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"
    [ -s "$TEST_TMP/ssh.log" ]
}

# ── fallback and opt-out (round 3 #5/#6, round 4 #5/#7) ──────────────────────
@test "gc_fallback_without_hygiene_keeps_a_dirty_worktree" {
    require_real_gh
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    export AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"
    wt="$(make_worktree wt-20 "feat/$CLOSED_ISSUE-fb")"
    echo wip > "$wt/wip.txt"
    run bash "$WATCHDOG"
    [ -f "$wt/wip.txt" ]
}

@test "gc_fallback_without_hygiene_removes_a_clean_pushed_worktree" {
    require_real_gh; require_real_autospec
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    wt="$(make_worktree wt-21 "feat/$CLOSED_ISSUE-fb2")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
}

@test "gc_fallback_keeps_edits_made_during_the_runtime_teardown" {
    require_real_gh
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    # a real adapter process that edits the worktree mid-teardown, as an adopting worker would
    printf '#!/usr/bin/env bash\necho edit > "$5/late.txt"\n' > "$TEST_TMP/editing-autospec"
    chmod +x "$TEST_TMP/editing-autospec"
    export AUTOSPEC_BIN="$TEST_TMP/editing-autospec"
    wt="$(make_worktree wt-22 "feat/$CLOSED_ISSUE-late")"
    run bash "$WATCHDOG"
    [ -f "$wt/late.txt" ]
}

@test "gc_cleanup_off_still_reclaims_via_the_pre-existing guarded removal" {
    require_real_gh; require_real_autospec
    export AUTOSPEC_DISK_CLEANUP=off
    wt="$(make_worktree wt-30 "feat/$CLOSED_ISSUE-off")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
    [[ "$output" == *"garbage_collected=1"* ]]
}

@test "gc_cleanup_off_still_keeps_a_dirty_worktree" {
    require_real_gh
    export AUTOSPEC_DISK_CLEANUP=off AUTOSPEC_BIN="$TEST_TMP/no-such-autospec"
    wt="$(make_worktree wt-31 "feat/$CLOSED_ISSUE-offd")"
    echo wip > "$wt/wip.txt"
    run bash "$WATCHDOG"
    [ -f "$wt/wip.txt" ]
}

@test "gc_number_slug_branch_is_issue_owned" {
    require_real_gh; require_real_autospec
    wt="$(make_worktree wt-s "feat/$CLOSED_ISSUE-released-heartbeat")"
    run bash "$WATCHDOG"
    [ ! -e "$wt" ]
}
