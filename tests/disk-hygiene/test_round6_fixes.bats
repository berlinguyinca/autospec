#!/usr/bin/env bats
# tests/disk-hygiene/test_round6_fixes.bats — regression tests for the round-6
# review findings. No fakes: real git repos, nested worktrees, the real scripts
# and the real gh (read-only, and only where a test truly needs GitHub).

load gc_helpers

setup() { gc_setup; }
teardown() { [ -z "${TEST_TMP:-}" ] || gc_teardown; }

# A clean, fully pushed linked worktree (branch at the pushed base) with an
# ignored nested-agent directory, the way Claude Code lays out isolated agents.
clean_outer() {
    OUTER="$(make_worktree outer "feat/42-outer")"
    ignore_nested "$OUTER"
}

ignore_nested() {
    local c; c="$(git -C "$1" rev-parse --path-format=absolute --git-common-dir)"
    mkdir -p "$c/info"; printf '.claude/worktrees/\n' >> "$c/info/exclude"
}

# ── #1 nested worktrees / repos are never removed with their parent ──────────
@test "cleanup keeps a worktree that holds a registered nested linked worktree with uncommitted work" {
    clean_outer
    git -C "$OUTER" worktree add -q -b nested-agent "$OUTER/.claude/worktrees/agent-a" "$BASE"
    echo wip > "$OUTER/.claude/worktrees/agent-a/wip.rs"
    [ -z "$(git -C "$OUTER" status --porcelain)" ]          # the trap: parent looks clean
    run bash "$DH" cleanup-worktree "$OUTER"
    [ "$status" -eq 1 ]
    [ -f "$OUTER/.claude/worktrees/agent-a/wip.rs" ]
    [[ "$output" == *"why=nested_worktree"* ]]
}

@test "cleanup --force still keeps a worktree with a nested worktree" {
    clean_outer
    git -C "$OUTER" worktree add -q -b nested-agent "$OUTER/.claude/worktrees/agent-a" "$BASE"
    echo wip > "$OUTER/.claude/worktrees/agent-a/wip.rs"
    run bash "$DH" cleanup-worktree --force "$OUTER"
    [ "$status" -eq 1 ]
    [ -f "$OUTER/.claude/worktrees/agent-a/wip.rs" ]
}

@test "keep-reason reports nested_worktree for a nested plain repo (a .git dir below the tree)" {
    clean_outer
    mkdir -p "$OUTER/.claude/worktrees/side"
    git init -q "$OUTER/.claude/worktrees/side"
    echo wip > "$OUTER/.claude/worktrees/side/wip.rs"
    run bash "$DH" keep-reason "$OUTER"
    [ "$output" = "nested_worktree" ]
    run bash "$DH" cleanup-worktree "$OUTER"
    [ "$status" -eq 1 ]
    [ -f "$OUTER/.claude/worktrees/side/wip.rs" ]
}

@test "a clean pushed worktree with no nested repo is still removed (control)" {
    clean_outer
    run bash "$DH" cleanup-worktree "$OUTER"
    [ "$status" -eq 0 ]
    [ ! -e "$OUTER" ]
}

@test "the watchdog GC keeps a closed-issue worktree holding a nested worktree" {
    require_real_gh
    require_real_autospec
    OUTER="$(make_worktree wt-outer "feat/$CLOSED_ISSUE-outer")"
    ignore_nested "$OUTER"
    git -C "$OUTER" worktree add -q -b nested-agent "$OUTER/.claude/worktrees/agent-a" "$BASE"
    echo wip > "$OUTER/.claude/worktrees/agent-a/wip.rs"
    run_gc
    [ -f "$OUTER/.claude/worktrees/agent-a/wip.rs" ]
    [[ "$output" == *"keeping "*"nested_worktree"* ]]
}

@test "the watchdog fallback (cleanup opted out) keeps a worktree holding a nested worktree" {
    require_real_gh
    require_real_autospec
    OUTER="$(make_worktree wt-outer "feat/$CLOSED_ISSUE-outer")"
    ignore_nested "$OUTER"
    git -C "$OUTER" worktree add -q -b nested-agent "$OUTER/.claude/worktrees/agent-a" "$BASE"
    echo wip > "$OUTER/.claude/worktrees/agent-a/wip.rs"
    export AUTOSPEC_DISK_CLEANUP=off
    run_gc
    [ -f "$OUTER/.claude/worktrees/agent-a/wip.rs" ]
    [[ "$output" == *"keeping "*"nested_worktree"* ]]
}

@test "the watchdog fallback fails closed and keeps the worktree when disk-hygiene.sh is missing" {
    require_real_gh
    require_real_autospec
    OUTER="$(make_worktree wt-outer "feat/$CLOSED_ISSUE-outer")"
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/no-such-disk-hygiene.sh"
    run_gc
    [ -d "$OUTER" ]
    [[ "$output" == *"keeping "* ]]
}

# ── #2 dirty checks ignore status.showUntrackedFiles ─────────────────────────
@test "keep-reason sees a new untracked file under status.showUntrackedFiles=no" {
    clean_outer
    git -C "$OUTER" config status.showUntrackedFiles no
    echo new > "$OUTER/new_module.rs"
    [ -z "$(git -C "$OUTER" status --porcelain)" ]          # the trap
    run bash "$DH" keep-reason "$OUTER"
    [ "$output" = "dirty" ]
    run bash "$DH" cleanup-worktree "$OUTER"
    [ "$status" -eq 1 ]
    [ -f "$OUTER/new_module.rs" ]
}

@test "_gc_unchanged_since sees an untracked file under status.showUntrackedFiles=no" {
    load_watchdog_fn _gc_unchanged_since
    DISK_HYGIENE_SH="$DH"
    clean_outer
    head="$(git -C "$OUTER" rev-parse HEAD)"
    git -C "$OUTER" config status.showUntrackedFiles no
    echo new > "$OUTER/new_module.rs"
    ! _gc_unchanged_since "$OUTER" "$head"
}

@test "the watchdog GC keeps a closed-issue worktree with an untracked file under showUntrackedFiles=no" {
    require_real_gh
    require_real_autospec
    OUTER="$(make_worktree wt-outer "feat/$CLOSED_ISSUE-outer")"
    git -C "$OUTER" config status.showUntrackedFiles no
    echo new > "$OUTER/new_module.rs"
    run_gc
    [ -f "$OUTER/new_module.rs" ]
    [[ "$output" == *"keeping "*"dirty"* ]]
}

@test "every dirty check uses the strict status flags" {
    for f in scripts/disk-hygiene.sh scripts/autospec-watchdog.sh scripts/worktree-guard.sh; do
        run bash -c "grep -n 'status --porcelain' '$ROOT/$f' | grep -v 'untracked-files=all' | grep -v '^[0-9]*: *#'"
        [ -z "$output" ] || { echo "$f: $output"; false; }
    done
}

@test "worktree-guard create refuses to reuse a path with an untracked file under showUntrackedFiles=no" {
    clean_outer
    git -C "$OUTER" config status.showUntrackedFiles no
    echo new > "$OUTER/new_module.rs"
    cd "$PRIMARY"
    run bash "$ROOT/scripts/worktree-guard.sh" create --branch feat/42-outer --path "$OUTER"
    [ "$status" -ne 0 ]
    [ -f "$OUTER/new_module.rs" ]
}

# ── #3 helpers ship together; a missing helper never disables the watchdog ──
@test "watchdog run without autospec-timeout.sh warns and still reaches gh (unbounded)" {
    require_real_gh
    mkdir -p "$TEST_TMP/lone" "$TEST_TMP/home"
    cp "$WATCHDOG" "$TEST_TMP/lone/autospec-watchdog.sh"
    HOME="$TEST_TMP/home" run bash "$TEST_TMP/lone/autospec-watchdog.sh"
    [[ "$output" == *"autospec-timeout.sh not found"* ]]
    [[ "$output" != *"skipping pass"* ]]
}

@test "disk-hygiene.sh run without autospec-timeout.sh warns instead of failing closed" {
    mkdir -p "$TEST_TMP/lone" "$TEST_TMP/home"
    cp "$DH" "$TEST_TMP/lone/disk-hygiene.sh"
    clean_outer
    HOME="$TEST_TMP/home" run bash "$TEST_TMP/lone/disk-hygiene.sh" keep-reason "$OUTER"
    [[ "$output" == *"autospec-timeout.sh not found"* ]]
}

@test "every per-skill installer that ships the watchdog also ships its sourced helpers" {
    n=0
    for f in "$ROOT"/skills/*/install.sh; do
        line="$(grep -E '^SHARED_SCRIPT_FILES=' "$f" || true)"
        case "$line" in *autospec-watchdog.sh*) ;; *) continue ;; esac
        n=$((n+1))
        for need in autospec-timeout.sh disk-hygiene.sh; do
            case "$line" in *"$need"*) ;; *) echo "$f missing $need"; false ;; esac
        done
    done
    [ "$n" -ge 10 ]
}

# ── #4 the squash-merge shortcut, against the real gh and real PRs ──────────
# Both PR heads are real commits of this repo; the local bare "origin" has every
# branch and tag containing them removed, so no git fact says they are pushed.
unpublish() {
    local oid ref
    for oid in "$MERGED_PR_OID" "$CLOSED_PR_OID"; do
        for ref in $(git -C "$ORIGIN" for-each-ref --contains "$oid" --format='%(refname)' refs/heads refs/tags); do
            git -C "$ORIGIN" update-ref -d "$ref"
        done
    done
    git -C "$PRIMARY" fetch -q --prune origin 2>/dev/null || true
}

@test "dh_pushed: a PR MERGED at exactly HEAD counts as pushed (real gh, real PR #1)" {
    require_real_gh
    unpublish
    # git rewrites origin to a local path (insteadOf), which gh cannot map to a
    # host, so name the repo the way gh supports: GH_REPO.
    export GH_REPO="$REAL_REPO"
    wt="$AUTOSPEC_WATCHDOG_GC_DIR/wt-merged"
    git -C "$PRIMARY" worktree add -q -b "$MERGED_PR_BRANCH" "$wt" "$MERGED_PR_OID"
    run bash "$DH" keep-reason "$wt"
    [ -z "$output" ]
    run bash "$DH" cleanup-worktree "$wt"
    [ "$status" -eq 0 ]
    [ ! -e "$wt" ]
}

@test "dh_pushed: a CLOSED unmerged PR at HEAD does NOT count as pushed (real gh, real PR #303)" {
    require_real_gh
    unpublish
    # git rewrites origin to a local path (insteadOf), which gh cannot map to a
    # host, so name the repo the way gh supports: GH_REPO.
    export GH_REPO="$REAL_REPO"
    wt="$AUTOSPEC_WATCHDOG_GC_DIR/wt-closed"
    git -C "$PRIMARY" worktree add -q -b "$CLOSED_PR_BRANCH" "$wt" "$CLOSED_PR_OID"
    run bash "$DH" keep-reason "$wt"
    [ "$output" = "unpushed" ]
    run bash "$DH" cleanup-worktree "$wt"
    [ "$status" -eq 1 ]
    [ -d "$wt" ]
}

# ── tests never write to the real repo ───────────────────────────────────────
@test "AUTOSPEC_WATCHDOG_DRY_RUN=1 makes reclaim, nudge and run-state clearing skip every GitHub write" {
    load_watchdog_fn _wd_dry_run clear_run_state_comments_for_issue reclaim_issue nudge_issue
    WATCHDOG_LOG_PREFIX="wd:"
    # gh must never reach GitHub from this test, even where gh lives in
    # /usr/bin: an unresolvable host and a dummy token make every call fail.
    export PATH="/usr/bin:/bin" GH_HOST=unreachable.invalid GH_TOKEN=x
    AUTOSPEC_WATCHDOG_DRY_RUN=1 run nudge_issue 7
    [ "$status" -eq 0 ]
    [[ "$output" == *"dry-run: skipping GitHub write: nudge #7"* ]]
    AUTOSPEC_WATCHDOG_DRY_RUN=1 run reclaim_issue 7 100
    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaim #7"* ]]
    AUTOSPEC_WATCHDOG_DRY_RUN=1 run clear_run_state_comments_for_issue 7
    [ "$status" -eq 0 ]
    # control: without the switch the write is attempted and fails (the host
    # cannot resolve, so nothing is ever posted)
    REPO_ARGS="--repo berlinguyinca/autospec" WATCHDOG_GC_GH_TIMEOUT_SECS=5
    autospec_timeout() { shift; "$@"; }
    run nudge_issue 7
    [ "$status" -ne 0 ]
}
