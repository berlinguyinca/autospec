#!/usr/bin/env bats
# tests/disk-hygiene/test_round5_fixes.bats — regression tests for the round-5
# review findings on the disk-hygiene PR. No fakes: real git repos and worktrees,
# the real watchdog / disk-hygiene / timeout scripts and the real gh (read-only;
# only the tests that truly need GitHub call require_real_gh). Decisions are pure
# functions fed with real data. Scratch lives under the gitignored .test-tmp/.

load gc_helpers

setup() { gc_setup; }
teardown() { [ -z "${TEST_TMP:-}" ] || gc_teardown; }

TIMEOUT_SH="$ROOT/scripts/autospec-timeout.sh"

# ── #1 candidates come from `git worktree list`, not a glob ──────────────────
@test "gc_sees_a_slash_branch_worktree_nested_like_dispatch_implementer (feat/autonomous-issue-42)" {
    wt="$(make_worktree wt-feat/autonomous-issue-42 feat/autonomous-issue-42)"
    commit_unpushed "$wt"
    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" == *"keeping "*"wt-feat/autonomous-issue-42 (issue #42): unpushed"* ]]
}

@test "_gc_candidates lists nested worktrees under wt-*, not the primary, not strangers" {
    load_watchdog_fn _gc_candidates
    a="$(make_worktree wt-feat/autonomous-issue-42 feat/autonomous-issue-42)"
    b="$(make_worktree wt-flat feat/7-flat)"
    git -C "$PRIMARY" worktree add -q -b feat/9-elsewhere "$TEST_TMP/elsewhere" "$BASE"
    WATCHDOG_GC_DIR="$AUTOSPEC_WATCHDOG_GC_DIR" run _gc_candidates
    [ "$status" -eq 0 ]
    real_a="$(cd "$a" && pwd -P)"; real_b="$(cd "$b" && pwd -P)"
    [[ "$output" == *"$real_a"$'\t'"feat/autonomous-issue-42"* ]]
    [[ "$output" == *"$real_b"$'\t'"feat/7-flat"* ]]
    [[ "$output" != *"elsewhere"* ]]
    [[ "$output" != *"$(cd "$PRIMARY" && pwd -P)"$'\t'* ]]
}

@test "gc_never_enumerates_another_repos_worktree" {
    mkdir -p "$TEST_TMP/acme"
    git init -q --bare "$TEST_TMP/acme/other.git"
    git clone -q "$TEST_TMP/acme/other.git" "$TEST_TMP/other-primary" 2>/dev/null
    git -C "$TEST_TMP/other-primary" checkout -q -b main
    echo s > "$TEST_TMP/other-primary/s.txt"; git -C "$TEST_TMP/other-primary" add -A
    git -C "$TEST_TMP/other-primary" commit -q -m s
    git -C "$TEST_TMP/other-primary" push -q -u origin main
    other="$(REPO_FOR_WT="$TEST_TMP/other-primary" make_worktree wt-x "feat/autonomous-issue-$CLOSED_ISSUE" origin/main)"
    mine="$(make_worktree wt-y "feat/$CLOSED_ISSUE-mine")"
    commit_unpushed "$mine"
    run_gc
    [ -d "$other" ]
    [[ "$output" == *"keeping "*"wt-y (issue #$CLOSED_ISSUE): unpushed"* ]]   # control: the loop ran
    [[ "$output" != *"wt-x"* ]]
}

# ── #4 ownership: heartbeat record or fixed autospec branch formats ──────────
@test "branch -> issue: autospec formats and heartbeat records only" {
    load_watchdog_fn _gc_issue_of_branch json_value
    WATCHDOG_DIR="$TEST_TMP/hb"; mkdir -p "$WATCHDOG_DIR"
    [ "$(_gc_issue_of_branch feat/autonomous-issue-42)" = 42 ]
    [ "$(_gc_issue_of_branch feat/2879-released-heartbeat)" = 2879 ]
    printf '{"issue":"700","branch":"custom/odd-name","ts":1}\n' > "$WATCHDOG_DIR/700.json"
    [ "$(_gc_issue_of_branch custom/odd-name)" = 700 ]
    for b in release/2026-q4 deps/2024-upgrade backup/20261118-nightly hotfix/1 fix/issue-12 \
             feat/autospec-issue-700 explore-qa-gate-12345 codex/foo feat/123456789012-too-long main; do
        [ -z "$(_gc_issue_of_branch "$b")" ]
    done
}

@test "gc_leaves_release_2026-q4_alone" {
    keep="$(make_worktree wt-keep feat/3-control)"; commit_unpushed "$keep"
    rel="$(make_worktree wt-release release/2026-q4)"
    run_gc
    [ -d "$rel" ]
    [[ "$output" == *"keeping "*"wt-keep (issue #3): unpushed"* ]]   # control: the loop ran
    [[ "$output" != *"wt-release"* ]]
}

# ── #5 ownership of the repo: host + owner/name of origin ────────────────────
@test "remote slug: host and owner/name from every URL form, empty for paths" {
    load_watchdog_fn _gc_remote_slug
    [ "$(_gc_remote_slug https://github.com/Acme/Api.git)" = github.com/acme/api ]
    [ "$(_gc_remote_slug https://user@github.com/acme/api/)" = github.com/acme/api ]
    [ "$(_gc_remote_slug git@github.com:acme/api.git)" = github.com/acme/api ]
    [ "$(_gc_remote_slug ssh://git@gitlab.example.com:2222/acme/api.git)" = gitlab.example.com/acme/api ]
    [ -z "$(_gc_remote_slug /srv/git/acme/api.git)" ]
    [ -z "$(_gc_remote_slug https://github.com/acme)" ]
}

@test "gc_skips_the_pass_when_origin_is_the_same_name_on_another_host" {
    wt="$(make_worktree wt-g feat/5-gitlab)"; commit_unpushed "$wt"
    git -C "$PRIMARY" config remote.origin.url "https://gitlab.example.com/$REAL_REPO.git"
    run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" == *"not this repo"* ]]
    [[ "$output" != *"keeping"* ]]
}

@test "gc_ignores_an_extra_remote_pointing_at_this_repo_when_origin_is_another_repo" {
    wt="$(make_worktree wt-h feat/6-subtree)"; commit_unpushed "$wt"
    git -C "$PRIMARY" remote add src "https://github.com/$REAL_REPO.git"
    git -C "$PRIMARY" config remote.origin.url "https://github.com/acme/api-docs.git"
    run_gc
    [ -d "$wt" ]
    [[ "$output" == *"not this repo"* ]]
}

# ── #2 one shared timeout helper, pure-bash fallback ─────────────────────────
@test "timeout fallback: a hung command is killed, rc 124, bounded" {
    . "$TIMEOUT_SH"
    start="$(date +%s)"
    run autospec_timeout_fallback 1 sleep 30
    [ "$status" -eq 124 ]
    [ $(( $(date +%s) - start )) -lt 10 ]
}

@test "timeout fallback: a grandchild holding the output pipe does not hang command substitution" {
    . "$TIMEOUT_SH"
    start="$(date +%s)"
    out="$(autospec_timeout_fallback 1 bash -c 'sleep 30 & echo partial; wait' || true)"
    [ "$out" = partial ]
    [ $(( $(date +%s) - start )) -lt 10 ]
}

@test "timeout fallback: output and exit status of a fast command pass through" {
    . "$TIMEOUT_SH"
    run autospec_timeout_fallback 5 sh -c 'echo hi; exit 3'
    [ "$status" -eq 3 ]
    [ "$output" = hi ]
}

@test "autospec_timeout works with and without timeout binaries" {
    . "$TIMEOUT_SH"
    run autospec_timeout 5 echo ok; [ "$output" = ok ]
    # PATH holding only the shell tools the helper needs, no timeout/gtimeout
    mkdir -p "$TEST_TMP/bin"
    for t in sleep kill pgrep echo sh bash date; do ln -sf "$(command -v $t)" "$TEST_TMP/bin/$t"; done
    PATH="$TEST_TMP/bin" run autospec_timeout 1 sleep 30
    [ "$status" -eq 124 ]
}

@test "every gh and git-fetch call in the watchdog and disk-hygiene scripts goes through autospec_timeout" {
    run bash -c "grep -nE '(^|[^_a-zA-Z.])gh (issue|pr|api|repo|auth)\b|git .*fetch' '$ROOT/scripts/autospec-watchdog.sh' '$ROOT/scripts/disk-hygiene.sh' \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*#' | grep -v autospec_timeout"
    echo "$output" >&2
    [ -z "$output" ]
}

# ── #9 ssh options follow git's own variant detection ────────────────────────
@test "dh_ssh_command: BatchMode only for OpenSSH variants" {
    . "$DH"
    o="-o BatchMode=yes -o ConnectTimeout=10"
    [ "$(dh_ssh_command 'ssh -i key' '')" = "ssh -i key $o" ]
    [ "$(dh_ssh_command '/usr/bin/ssh -i key' auto)" = "/usr/bin/ssh -i key $o" ]
    [ "$(dh_ssh_command 'ssh.exe' '')" = "ssh.exe $o" ]
    [ "$(dh_ssh_command '/opt/x/odd-wrapper' ssh)" = "/opt/x/odd-wrapper $o" ]
    [ "$(dh_ssh_command 'plink -batch' '')" = "plink -batch" ]
    [ "$(dh_ssh_command 'tortoiseplink' auto)" = "tortoiseplink" ]
    [ "$(dh_ssh_command '/usr/local/bin/ssh-deploy' '')" = "/usr/local/bin/ssh-deploy" ]
    [ "$(dh_ssh_command 'ssh' putty)" = "ssh" ]
    [ "$(dh_ssh_command 'ssh' simple)" = "ssh" ]
    [ "$(dh_ssh_command '' '')" = "ssh $o" ]
}

@test "dh_fetch prunes a branch deleted on the remote (real local transport)" {
    git -C "$PRIMARY" push -q origin "$(git -C "$PRIMARY" rev-parse HEAD):refs/heads/gone-soon"
    git -C "$PRIMARY" fetch -q origin
    git -C "$PRIMARY" rev-parse -q --verify origin/gone-soon >/dev/null
    git --git-dir="$ORIGIN" branch -q -D gone-soon
    bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"
    ! git -C "$PRIMARY" rev-parse -q --verify origin/gone-soon >/dev/null
}

@test "dh_fetch over a real ssh to localhost (only when sshd accepts batch logins)" {
    ssh -o BatchMode=yes -o ConnectTimeout=3 localhost true >/dev/null 2>&1 || skip "no passwordless ssh to localhost"
    git -C "$PRIMARY" config remote.origin.url "ssh://localhost$ORIGIN"
    git --git-dir="$ORIGIN" branch -q gone-soon "$(git --git-dir="$ORIGIN" rev-parse HEAD)"
    bash -c '. "$1"; dh_fetch "$2"' _ "$DH" "$PRIMARY"
    git -C "$PRIMARY" rev-parse -q --verify origin/gone-soon >/dev/null
}

# ── #3 gone issues are permanent, transient errors are not ───────────────────
load_issue_meta() { . "$TIMEOUT_SH"; load_watchdog_fn issue_meta; WATCHDOG_GC_GH_TIMEOUT_SECS=30; REPO_ARGS="--repo $REAL_REPO"; }

@test "issue_meta: a nonexistent issue is GONE (real gh)" {
    require_real_gh; load_issue_meta
    [ "$(issue_meta 999999999)" = GONE ]
}

@test "issue_meta: an unreachable host is transient (empty), not GONE (real gh)" {
    command -v gh >/dev/null || skip "gh not installed"; load_issue_meta
    [ -z "$(GH_HOST=unreachable.invalid GH_TOKEN=x issue_meta 297)" ]
}

@test "issue_meta: a real closed issue reports its state (real gh)" {
    require_real_gh; load_issue_meta
    [[ "$(issue_meta "$CLOSED_ISSUE")" == "CLOSED "* ]]
}

@test "main loop removes the heartbeat and nudge state of a deleted issue (real gh)" {
    require_real_gh
    unset AUTOSPEC_WATCHDOG_GC_ONLY
    # Reads from the real repo (the deleted-issue lookup); every GitHub write path
    # (reclaim, nudge, labels, comments) is dry-run, so nothing here mutates it.
    export AUTOSPEC_WATCHDOG_DRY_RUN=1
    HB="$AUTOSPEC_WATCHDOG_DIR/berlinguyinca__autospec/999999999.json"
    printf '{"issue":"999999999","branch":"feat/999999999-x","step":"tests_started","ts":%s,"pr":"","repo":"%s","host":"%s"}\n' \
        "$(date -u +%s)" "$REAL_REPO" "$AUTOSPEC_HOST" > "$HB"
    export AUTOSPEC_WATCHDOG_STATE_FILE="$TEST_TMP/state.tsv"
    printf '999999999\t1\n' > "$AUTOSPEC_WATCHDOG_STATE_FILE"
    run bash "$WATCHDOG"
    [ "$status" -eq 0 ]
    [ ! -e "$HB" ]
    ! grep -q '^999999999' "$AUTOSPEC_WATCHDOG_STATE_FILE"
}

@test "gc reclaims a clean pushed worktree of a deleted issue (real gh, real adapter)" {
    require_real_gh; require_real_autospec
    wt="$(make_worktree wt-feat/999999999-gone feat/999999999-gone)"
    run_gc
    [ ! -e "$wt" ]
    [[ "$output" == *"garbage_collected=1"* ]]
}

# ── #6 the dirty-tree guard really ran ───────────────────────────────────────
@test "gc_fallback_dirty_guard_runs_before_any_runtime_teardown" {
    export AUTOSPEC_DISK_HYGIENE_SH="$TEST_TMP/missing-hygiene.sh"
    wt="$(make_worktree wt-fd feat/8-fb)"
    echo wip > "$wt/wip.txt"
    run_gc
    [ -f "$wt/wip.txt" ]
    [[ "$output" == *"keeping "*"wt-fd (issue #8): dirty"* ]]
    [[ "$output" != *"runtime cleanup"* ]]
}

@test "gc_cleanup_off_dirty_guard_runs_before_any_runtime_teardown" {
    export AUTOSPEC_DISK_CLEANUP=off
    wt="$(make_worktree wt-od feat/8-off)"
    echo wip > "$wt/wip.txt"
    run_gc
    [ -f "$wt/wip.txt" ]
    [[ "$output" == *"keeping "*"wt-od (issue #8): dirty"* ]]
    [[ "$output" != *"runtime cleanup"* ]]
}

# ── #8 the teardown-window re-check is a pure decision over a real worktree ──
@test "_gc_unchanged_since: true for an untouched worktree, false after an edit or a commit" {
    load_watchdog_fn _gc_unchanged_since
    wt="$(make_worktree wt-u feat/10-u)"
    head="$(git -C "$wt" rev-parse HEAD)"
    _gc_unchanged_since "$wt" "$head"
    echo late > "$wt/late.txt"
    ! _gc_unchanged_since "$wt" "$head"
    rm "$wt/late.txt"
    _gc_unchanged_since "$wt" "$head"
    commit_unpushed "$wt"
    ! _gc_unchanged_since "$wt" "$head"
}

# ── #10 docs match behaviour ─────────────────────────────────────────────────
@test "docs and script header describe cleanup-enabled and the watchdog GC opt-out truthfully" {
    grep -q 'cleanup-enabled' "$DH"
    grep -q 'cleanup-enabled' "$ROOT/docs/disk-hygiene.md"
    ! grep -q 'AUTOSPEC_DISK_CLEANUP=off  *no automatic worktree cleanup' "$DH"
    ! grep -qE '^\| Everything \| \| `AUTOSPEC_DISK_HYGIENE=off` \|' "$ROOT/docs/disk-hygiene.md"
}
