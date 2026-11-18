#!/usr/bin/env bats
# tests/disk-hygiene/test_round3_fixes.bats — regression tests for the round-3
# review findings on the disk-hygiene PR. Real git repos, real bare remotes,
# real linked worktrees, real processes (the real watchdog and disk-hygiene
# scripts run). All scratch lives under the gitignored .test-tmp/ in the repo.
#
# The only stand-in is a PATH `gh`: GitHub's API has no local equivalent (gh
# always speaks TLS to a hosted API), so the stub sits exactly at the process
# boundary of that external service and carries no logic beyond canned answers.

ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
DH="$ROOT/scripts/disk-hygiene.sh"
WATCHDOG="$ROOT/scripts/autospec-watchdog.sh"

setup() {
    mkdir -p "$ROOT/.test-tmp"
    TEST_TMP="$(mktemp -d "$ROOT/.test-tmp/r3.XXXXXX")"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    unset CARGO_TARGET_DIR CARGO_INCREMENTAL AUTOSPEC_DISK_HYGIENE \
          AUTOSPEC_DISK_CLEANUP AUTOSPEC_DISK_PREFLIGHT GIT_SSH_COMMAND
    export AUTOSPEC_HERDR_ENG="$TEST_TMP/no-such-herdr-eng"
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="o/n"
    export AUTOSPEC_WATCHDOG_GC_ONLY="1"
    export AUTOSPEC_WATCHDOG_GC_DIR="$TEST_TMP/wt"
    export AUTOSPEC_HOST="host-A"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/o_n" "$AUTOSPEC_WATCHDOG_GC_DIR"

    ORIGIN="$TEST_TMP/o/n.git"          # origin URL ends in o/n => repo o/n
    PRIMARY="$TEST_TMP/primary"
    mkdir -p "$TEST_TMP/o"
    git init -q --bare "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY" 2>/dev/null
    git -C "$PRIMARY" checkout -q -b main
    echo seed > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add -A
    git -C "$PRIMARY" commit -q -m seed
    git -C "$PRIMARY" push -q -u origin main
    git -C "$PRIMARY" remote set-head origin main >/dev/null

    export RUNTIME_CLEANUP_LOG="$TEST_TMP/runtime-cleanup.log"
    : > "$RUNTIME_CLEANUP_LOG"
    export GH_ISSUE_STATE="CLOSED" GH_ISSUE_LABELS="" GH_PR_STATE="" GH_PR_OID=""
    MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"
    write_gh_stub
    export AUTOSPEC_RUNTIME_WORKTREE_CLEANUP="$MOCK_DIR/runtime-cleanup.sh"
    printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$1" >> "$RUNTIME_CLEANUP_LOG"\n' > "$AUTOSPEC_RUNTIME_WORKTREE_CLEANUP"
    chmod +x "$AUTOSPEC_RUNTIME_WORKTREE_CLEANUP"
}

teardown() { rm -rf "$TEST_TMP"; }

write_gh_stub() {
    cat > "$MOCK_DIR/gh" <<'GEOF'
#!/usr/bin/env bash
case "$1 $2" in
  "issue view") sleep "${GH_ISSUE_DELAY:-0}"; printf '%s %s\n' "$GH_ISSUE_STATE" "$GH_ISSUE_LABELS" ;;
  "pr list") [ -n "$GH_PR_STATE" ] && printf '%s %s\n' "$GH_PR_STATE" "$GH_PR_OID" ;;
  "repo view") echo "o/n" ;;
esac
exit 0
GEOF
    chmod +x "$MOCK_DIR/gh"
}

make_worktree() { # name branch [repo]
    local dir="$AUTOSPEC_WATCHDOG_GC_DIR/$1"
    git -C "${3:-$PRIMARY}" worktree add -q -b "$2" "$dir" origin/main
    printf '%s' "$dir"
}

# 1. a worktree of ANOTHER repo with a colliding issue number is never touched.
@test "gc_other_repos_worktree_is_left_alone (colliding issue number)" {
    mkdir -p "$TEST_TMP/x/other"
    git init -q --bare "$TEST_TMP/x/other/repo.git"
    git clone -q "$TEST_TMP/x/other/repo.git" "$TEST_TMP/other-primary" 2>/dev/null
    git -C "$TEST_TMP/other-primary" checkout -q -b main
    echo s > "$TEST_TMP/other-primary/s.txt"
    git -C "$TEST_TMP/other-primary" add -A
    git -C "$TEST_TMP/other-primary" commit -q -m s
    git -C "$TEST_TMP/other-primary" push -q -u origin main
    other="$(make_worktree wt-12 feat/autospec-issue-12 "$TEST_TMP/other-primary")"
    mine="$(make_worktree wt-13 feat/autospec-issue-13)"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [ -d "$other" ]
    [ ! -e "$mine" ]
    ! grep -Fxq "$other" "$RUNTIME_CLEANUP_LOG"
}

# 2. cleanup-worktree --expect-head keeps a worktree whose HEAD moved.
@test "cleanup_expect_head_keeps_a_worktree_whose_head_changed" {
    wt="$(make_worktree wt-a feat/a)"
    old="$(git -C "$wt" rev-parse HEAD)"
    echo c > "$wt/c.txt"; git -C "$wt" add c.txt; git -C "$wt" commit -q -m local

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

# 3. A slow gh must not make the main heartbeat loop delete a live heartbeat.
@test "main_loop_slow_gh_does_not_delete_a_live_heartbeat" {
    unset AUTOSPEC_WATCHDOG_GC_ONLY
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement,in-progress-by-bot"
    export GH_ISSUE_DELAY=3 AUTOSPEC_WATCHDOG_GC_GH_TIMEOUT_SECS=1
    hb="$AUTOSPEC_WATCHDOG_DIR/o_n/900.json"
    printf '{"issue":"900","branch":"feat/autospec-issue-900","step":"tests_started","ts":%s,"pr":"","repo":"o/n","host":"host-A"}\n' \
        "$(date -u +%s)" > "$hb"

    run bash "$WATCHDOG"

    [ -f "$hb" ]
}

# 4. dh_fetch honours the repo's core.sshCommand.
@test "dh_fetch_respects_core_sshCommand" {
    cat > "$TEST_TMP/myssh" <<EOS
#!/usr/bin/env bash
echo called >> "$TEST_TMP/ssh.log"
last=""; for a in "\$@"; do last="\$a"; done
exec sh -c "\$last"
EOS
    chmod +x "$TEST_TMP/myssh"
    git -C "$PRIMARY" remote set-url origin "ssh://nohost$ORIGIN"
    git -C "$PRIMARY" config core.sshCommand "$TEST_TMP/myssh"

    bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"

    [ -s "$TEST_TMP/ssh.log" ]
}

# 5. Without disk-hygiene.sh the fallback still refuses a dirty tree.
@test "gc_fallback_without_hygiene_keeps_a_dirty_worktree" {
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    wt="$(make_worktree wt-20 feat/autospec-issue-20)"
    echo wip > "$wt/wip.txt"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [ -f "$wt/wip.txt" ]
}

@test "gc_fallback_without_hygiene_removes_a_clean_pushed_worktree" {
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    wt="$(make_worktree wt-21 feat/autospec-issue-21)"
    run bash "$WATCHDOG"
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

# 6. AUTOSPEC_DISK_CLEANUP=off: no teardown, nothing counted.
@test "gc_cleanup_off_does_no_teardown_and_counts_nothing" {
    export AUTOSPEC_DISK_CLEANUP=off
    wt="$(make_worktree wt-30 feat/autospec-issue-30)"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
    [[ "$output" == *"garbage_collected=0"* ]]
}

# 8. Autospec branches named <type>/<N>-slug are issue-owned too.
@test "gc_number_slug_branch_is_issue_owned" {
    wt="$(make_worktree wt-2879 feat/2879-released-heartbeat)"
    run bash "$WATCHDOG"
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

@test "gc_explore_qa_gate_branch_still_left_alone" {
    wt="$(make_worktree wt-g explore-qa-gate-12345)"
    run bash "$WATCHDOG"
    [ -d "$wt" ]
}
