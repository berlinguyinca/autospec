#!/usr/bin/env bats
# tests/resume/test_watchdog_gc.bats — watchdog orphaned-worktree GC pass.
#
# Covers (docs/specs/2026-06-03-crash-resume-design.md §Error handling "GC
# safety", §Decomposition Child 3): a /tmp/wt-* worktree is pruned via
# `git worktree remove --force` ONLY when ALL THREE hold:
#   (1) no un-pushed commits   — `git -C <wt> log --not --remotes` is empty
#   (2) issue closed/unlabeled — gh shows it not OPEN+labeled auto-implement
#   (3) no live heartbeat      — no fresh heartbeat references the branch
#
# Data-integrity contract (load-bearing): the GC must NEVER remove a worktree
# with un-pushed commits, and NEVER remove an in-progress-this-host worktree
# with a fresh heartbeat. Pruning is `git worktree remove --force` only — never
# `rm -rf`.
#
# Real git throughout: a bare origin, a primary clone, real linked worktrees
# under the GC scan root. The only stand-ins are a PATH `gh` (the GitHub API has
# no local equivalent) and the broker runtime-cleanup recorder. The real
# watchdog script runs in GC-only mode (AUTOSPEC_WATCHDOG_GC_ONLY=1).

ROOT="${BATS_TEST_DIRNAME}/../.."
WATCHDOG="$ROOT/scripts/autospec-watchdog.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"

    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="o/n"
    export AUTOSPEC_WATCHDOG_GC_ONLY="1"
    export AUTOSPEC_WATCHDOG_GC_DIR="$TEST_TMP/wt"   # scan root for /tmp/wt-* analogues
    export AUTOSPEC_HOST="host-A"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
    export AUTOSPEC_HERDR_ENG="$TEST_TMP/no-such-herdr-eng"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/o_n" "$AUTOSPEC_WATCHDOG_GC_DIR"

    ORIGIN="$TEST_TMP/origin.git"
    PRIMARY="$TEST_TMP/primary"
    git init -q --bare "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY" 2>/dev/null
    git -C "$PRIMARY" checkout -q -b main
    echo seed > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add -A
    git -C "$PRIMARY" commit -q -m seed
    git -C "$PRIMARY" push -q -u origin main

    export RUNTIME_CLEANUP_LOG="$TEST_TMP/runtime-cleanup.log"
    : > "$RUNTIME_CLEANUP_LOG"
    export GH_PR_LOG="$TEST_TMP/gh-pr.log"
    : > "$GH_PR_LOG"
    export GH_ISSUE_STATE="CLOSED"   # state of the issue
    export GH_ISSUE_LABELS=""        # comma-joined labels
    export GH_PR_STATE=""            # state of the branch's PR ("" = none)
    export GH_PR_OID=""              # PR head commit

    export MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"

    write_hostname_mock
    write_gh_mock
    write_runtime_cleanup_mock
}

teardown() { rm -rf "$TEST_TMP"; }

now_ts() { date -u +%s; }

write_hostname_mock() {
    cat > "$MOCK_DIR/hostname" <<HEOF
#!/usr/bin/env bash
echo "${AUTOSPEC_HOST}"
HEOF
    chmod +x "$MOCK_DIR/hostname"
}

# GitHub API stand-in (the one unavoidable stub).
write_gh_mock() {
    cat > "$MOCK_DIR/gh" <<'GEOF'
#!/usr/bin/env bash
case "$1 $2" in
  "issue view")
    printf '%s %s\n' "$GH_ISSUE_STATE" "$GH_ISSUE_LABELS"
    ;;
  "pr list")
    case "$*" in
      *"--state all"*)
        echo "pr-list" >> "$GH_PR_LOG"
        [ -n "$GH_PR_STATE" ] && printf '%s %s\n' "$GH_PR_STATE" "$GH_PR_OID"
        ;;
    esac
    ;;
  "repo view") echo "o/n" ;;
  *) exit 0 ;;
esac
exit 0
GEOF
    chmod +x "$MOCK_DIR/gh"
}

# Records what the broker runtime-cleanup adapter was asked to release.
write_runtime_cleanup_mock() {
    export AUTOSPEC_RUNTIME_WORKTREE_CLEANUP="$MOCK_DIR/autospec-runtime-worktree-cleanup.sh"
    cat > "$AUTOSPEC_RUNTIME_WORKTREE_CLEANUP" <<'REOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$RUNTIME_CLEANUP_LOG"
exit "${RUNTIME_CLEANUP_EXIT:-0}"
REOF
    chmod +x "$AUTOSPEC_RUNTIME_WORKTREE_CLEANUP"
}

# Create a real linked worktree <GC_DIR>/<name> on branch <branch> (from origin/main).
make_worktree() {
    local name="$1" branch="$2" dir
    dir="$AUTOSPEC_WATCHDOG_GC_DIR/$name"
    git -C "$PRIMARY" worktree add -q -b "$branch" "$dir" origin/main
    printf '%s' "$dir"
}

# Commit work in a worktree that is NOT pushed anywhere.
commit_unpushed() { echo w > "$1/w.txt"; git -C "$1" add w.txt; git -C "$1" commit -q -m w; }

# Write a heartbeat referencing a branch with a given ts and host.
write_heartbeat() {
    local issue="$1" branch="$2" ts="$3" host="$4"
    printf '{"issue":"%s","branch":"%s","step":"tests_started","ts":%s,"pr":"","repo":"o/n","host":"%s"}\n' \
        "$issue" "$branch" "$ts" "$host" \
        > "$AUTOSPEC_WATCHDOG_DIR/o_n/${issue}.json"
}

run_gc() { run bash "$WATCHDOG"; }

@test "gc_unpushed_not_pruned: a worktree with un-pushed commits is NOT removed" {
    wt="$(make_worktree wt-700 feat/autospec-issue-700)"
    commit_unpushed "$wt"

    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
}

@test "gc_in_progress_not_pruned: an in-progress-this-host worktree with a fresh heartbeat is NOT removed" {
    wt="$(make_worktree wt-701 feat/autospec-issue-701)"
    export GH_ISSUE_STATE="OPEN"
    export GH_ISSUE_LABELS="auto-implement,in-progress-by-bot"
    write_heartbeat 701 "feat/autospec-issue-701" "$(now_ts)" "host-A"

    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
}

@test "gc_clean_closed_pruned: clean + closed/unlabeled issue + no live heartbeat IS removed after runtime cleanup" {
    wt="$(make_worktree wt-702 feat/autospec-issue-702)"
    real="$(cd "$wt" && pwd -P)"

    run_gc
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
    [ "$(cat "$RUNTIME_CLEANUP_LOG")" = "$wt" ]
    run git -C "$PRIMARY" worktree list --porcelain
    [[ "$output" != *"$real"* ]]
    [ ! -s "$GH_PR_LOG" ]   # closed issue: no PR lookup needed
}

@test "gc_runtime_cleanup_failure_preserves_the_worktree" {
    wt="$(make_worktree wt-703 feat/autospec-issue-703)"
    export RUNTIME_CLEANUP_EXIT=23

    run_gc

    [ "$status" -eq 0 ]
    grep -Fxq "$wt" "$RUNTIME_CLEANUP_LOG"
    [ -d "$wt" ]
}

# Finding 3: the keep-check precedes teardown.
@test "gc_dirty_worktree_keeps_runtime_resources (no teardown before the keep-check)" {
    wt="$(make_worktree wt-704 feat/autospec-issue-704)"
    echo dirty > "$wt/dirty.txt"

    run_gc

    [ "$status" -eq 0 ]
    [ -f "$wt/dirty.txt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
}

@test "gc_unpushed_worktree_keeps_runtime_resources (no teardown before the keep-check)" {
    wt="$(make_worktree wt-705 feat/autospec-issue-705)"
    commit_unpushed "$wt"

    run_gc

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
}

# Finding 4: an open+active issue with a CLOSED PR is a retry, not an orphan.
@test "gc_open_active_closed_pr_within_grace_is_kept" {
    wt="$(make_worktree wt-706 feat/autospec-issue-706)"
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement"
    export GH_PR_STATE="CLOSED" GH_PR_OID="$(git -C "$wt" rev-parse HEAD)"

    run_gc

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ ! -s "$RUNTIME_CLEANUP_LOG" ]
}

@test "gc_open_active_closed_pr_after_grace_is_reclaimed" {
    wt="$(make_worktree wt-707 feat/autospec-issue-707)"
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement"
    export GH_PR_STATE="CLOSED" GH_PR_OID="$(git -C "$wt" rev-parse HEAD)"
    export AUTOSPEC_WATCHDOG_GC_PR_CLOSED_GRACE_SECS=0

    run_gc

    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

@test "gc_open_active_closed_pr_for_a_different_head_is_kept" {
    wt="$(make_worktree wt-708 feat/autospec-issue-708)"
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement"
    export GH_PR_STATE="CLOSED" GH_PR_OID="0000000000000000000000000000000000000000"
    export AUTOSPEC_WATCHDOG_GC_PR_CLOSED_GRACE_SECS=0

    run_gc

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
}

@test "gc_open_active_merged_pr_at_head_is_reclaimed" {
    wt="$(make_worktree wt-709 feat/autospec-issue-709)"
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement"
    export GH_PR_STATE="MERGED" GH_PR_OID="$(git -C "$wt" rev-parse HEAD)"

    run_gc

    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

@test "gc_open_active_with_no_pr_is_kept" {
    wt="$(make_worktree wt-710 feat/autospec-issue-710)"
    export GH_ISSUE_STATE="OPEN" GH_ISSUE_LABELS="auto-implement"

    run_gc

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
}
