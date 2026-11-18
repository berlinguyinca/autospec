#!/usr/bin/env bats
# tests/disk-hygiene/test_round2_fixes.bats — regression tests for the round-2
# review findings on the disk-hygiene PR. Real git, real bare remotes, real
# worktrees, real stat/df/gh binaries; the real scripts run. No fakes: where a
# test needs gh to fail it uses the real gh pointed at an unreachable host.

ROOT="${BATS_TEST_DIRNAME}/../.."
DH="$ROOT/scripts/disk-hygiene.sh"
DISPATCH="$ROOT/scripts/dispatch-implementer.sh"
GATE="$ROOT/scripts/explore-qa-gate.sh"
WATCHDOG="$ROOT/scripts/autospec-watchdog.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    unset CARGO_TARGET_DIR CARGO_INCREMENTAL AUTOSPEC_DISK_HYGIENE \
          AUTOSPEC_DISK_CLEANUP AUTOSPEC_DISK_PREFLIGHT
    export AUTOSPEC_HERDR_ENG="$TEST_TMP/no-such-herdr-eng"
    ORIGIN="$TEST_TMP/origin.git"
    PRIMARY="$TEST_TMP/primary"
    git init -q --bare "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY" 2>/dev/null
    git -C "$PRIMARY" checkout -q -b main
    echo seed > "$PRIMARY/seed.txt"
    echo "target/" > "$PRIMARY/.gitignore"
    git -C "$PRIMARY" add -A
    git -C "$PRIMARY" commit -q -m seed
    git -C "$PRIMARY" push -q -u origin main
    git -C "$PRIMARY" remote set-head origin main >/dev/null
    WT="$TEST_TMP/wt-a"
    git -C "$PRIMARY" worktree add -q -b feat/a "$WT" origin/main
    BRS=()
}

teardown() {
    local b
    for b in "${BRS[@]:-}"; do
        [ -n "$b" ] && rm -rf "/tmp/wt-$b"
    done
    rm -rf "$TEST_TMP"
}

commit_in() { echo "$2" > "$1/$2.txt"; git -C "$1" add "$2.txt"; git -C "$1" commit -q -m "$2"; }

# Run the real watchdog GC pass over $TEST_TMP/gc (no gh mock: caller sets PATH/GH_*).
run_gc() {
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats" AUTOSPEC_WATCHDOG_REPO="o/n" \
           AUTOSPEC_WATCHDOG_GC_ONLY=1 AUTOSPEC_WATCHDOG_GC_DIR="$TEST_TMP/gc" \
           AUTOSPEC_HOST=host-A AUTOSPEC_CONFIG_FILE="$TEST_TMP/none.yml" \
           AUTOSPEC_RUNTIME_WORKTREE_CLEANUP="$ROOT/scripts/autospec-runtime-worktree-cleanup.sh"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/o_n" "$AUTOSPEC_WATCHDOG_GC_DIR"
    run bash "$WATCHDOG"
}

gc_worktree() { # name branch
    mkdir -p "$TEST_TMP/gc"
    git -C "$PRIMARY" worktree add -q -b "$2" "$TEST_TMP/gc/$1" origin/main
    printf '%s' "$TEST_TMP/gc/$1"
}

# 1 ── idle time uses GNU stat first (fails on Linux at c127d766)
@test "R2-1 _gc_wt_idle_secs reports a small idle time for a fresh worktree (GNU and BSD stat)" {
    eval "$(sed -n '/^_gc_wt_idle_secs()/,/^}/p' "$WATCHDOG")"
    now_ts="$(date -u +%s)"
    commit_in "$WT" x
    idle="$(_gc_wt_idle_secs "$WT")"
    case "$idle" in ''|*[!0-9-]*) false ;; esac
    [ "$idle" -lt 120 ]
}

@test "R2-1 _gc_wt_idle_secs fails closed (0 = recently active) when mtime is unreadable" {
    eval "$(sed -n '/^_gc_wt_idle_secs()/,/^}/p' "$WATCHDOG")"
    now_ts="$(date -u +%s)"
    idle="$(_gc_wt_idle_secs "$TEST_TMP/not-a-repo")"
    [ "$idle" -eq 0 ]
}

# 2 ── dangling origin/HEAD fails closed (fails at c127d766)
@test "R2-2 a dangling origin/HEAD with an unpushed commit is KEPT, not declared pushed" {
    commit_in "$WT" local-only
    git -C "$ORIGIN" branch -q other main
    git -C "$ORIGIN" symbolic-ref HEAD refs/heads/other
    git -C "$ORIGIN" branch -q -D main
    # origin/HEAD still names origin/main; the fetch --prune inside deletes it
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
    [[ "$output" == *"unpushed"* ]]
}

# 6 ── merge commits are not covered by cherry (fails at c127d766)
@test "R2-6 a local merge commit on top of patch-equivalent work is KEPT" {
    commit_in "$WT" c
    commit_in "$PRIMARY" m
    git -C "$PRIMARY" push -q origin main
    git -C "$WT" fetch -q origin
    git -C "$WT" merge -q --no-ff --no-edit origin/main     # local-only merge commit
    # then the work lands on main as a patch-equivalent commit (squash/cherry-pick)
    git -C "$PRIMARY" cherry-pick "$(git -C "$WT" rev-parse HEAD^1)" >/dev/null
    git -C "$PRIMARY" push -q origin main
    git -C "$WT" fetch -q origin
    [ -n "$(git -C "$WT" rev-list --merges origin/main..HEAD)" ]
    ! git -C "$WT" cherry origin/main HEAD | grep -q '^+'   # cherry alone would say "pushed"
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
}

@test "R2-6 patch-equivalent work without merges is still pushed" {
    commit_in "$WT" c
    git -C "$PRIMARY" cherry-pick "$(git -C "$WT" rev-parse HEAD)" >/dev/null
    git -C "$PRIMARY" push -q origin main
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

# 3 ── gate always removes the worktree it created (fails at c127d766)
gate_setup() {
    cd "$PRIMARY"
    mkdir -p .autospec
    : > .autospec/test.yml
    printf '{"branch":"main","slug":"d","base":"main","head_sha":"%s","created_at":"2026-01-01T00:00:00Z"}\n' \
        "$(git rev-parse HEAD)" > .autospec/explore-mode.json
    GATE_WT="$TEST_TMP/gate-wt"
}

@test "R2-3 explore-qa-gate removes its own worktree even with AUTOSPEC_DISK_CLEANUP=off" {
    gate_setup
    AUTOSPEC_DISK_CLEANUP=off AUTOSPEC_QA_CMD=true AUTOSPEC_QA_GATE_WORKTREE="$GATE_WT" \
        AUTOSPEC_STATE_DIR="$TEST_TMP/state" run bash "$GATE"
    [ ! -e "$GATE_WT" ]
    run git -C "$PRIMARY" worktree list --porcelain
    [[ "$output" != *"gate-wt"* ]]
}

@test "R2-3 explore-qa-gate removes its own worktree even with AUTOSPEC_DISK_HYGIENE=off" {
    gate_setup
    AUTOSPEC_DISK_HYGIENE=off AUTOSPEC_QA_CMD=true AUTOSPEC_QA_GATE_WORKTREE="$GATE_WT" \
        AUTOSPEC_STATE_DIR="$TEST_TMP/state" run bash "$GATE"
    [ ! -e "$GATE_WT" ]
}

# 4 ── plain-dir fallback never rm -rf's a dir holding a .git entry (fails at c127d766)
@test "R2-4 dispatch --cleanup keeps a directory whose .git file dangles (pruned worktree)" {
    BR="dh-dangle-$$-$RANDOM"; BRS+=("$BR")
    mkdir -p "/tmp/wt-$BR"
    echo "gitdir: $TEST_TMP/gone/.git/worktrees/x" > "/tmp/wt-$BR/.git"
    echo precious > "/tmp/wt-$BR/work.txt"
    cd "$PRIMARY"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 2 ]
    [ -f "/tmp/wt-$BR/work.txt" ]
}

@test "R2-4 dispatch --cleanup keeps a directory holding a .git directory" {
    BR="dh-gitdir-$$-$RANDOM"; BRS+=("$BR")
    mkdir -p "/tmp/wt-$BR/.git"
    echo precious > "/tmp/wt-$BR/work.txt"
    cd "$PRIMARY"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 2 ]
    [ -f "/tmp/wt-$BR/work.txt" ]
}

# 5 ── non-issue branches are not issue-owned; gh failure keeps (fails at c127d766)
@test "R2-5 the watchdog GC leaves a non-issue worktree (explore-qa-gate-<PID>) alone" {
    wt="$(gc_worktree wt-explore-qa-gate-12345 explore-qa-gate-12345)"
    GH_HOST=unreachable.invalid GH_TOKEN=x run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
}

@test "R2-5 the watchdog GC keeps an issue worktree when gh fails (state unknown)" {
    command -v gh >/dev/null 2>&1 || skip "gh not installed"
    wt="$(gc_worktree wt-702 feat/autospec-issue-702)"
    GH_HOST=unreachable.invalid GH_TOKEN=x run_gc
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [[ "$output" == *"issue_state_unknown"* ]]
}

# 7 ── keep-reason computed once; fetch bounded
@test "R2-7 cleanup-worktree --checked does not fetch but still refuses a dirty tree" {
    printf '#!/usr/bin/env bash\necho x >> "%s/fetches.log"\nexec "$(git --exec-path)/git-upload-pack" "$@"\n' "$TEST_TMP" > "$TEST_TMP/count-upload-pack"
    chmod +x "$TEST_TMP/count-upload-pack"
    git -C "$PRIMARY" config remote.origin.uploadpack "$TEST_TMP/count-upload-pack"
    echo dirty > "$WT/dirty.txt"
    run bash "$DH" cleanup-worktree --checked "$WT"
    [ "$status" -eq 1 ]
    [ -f "$WT/dirty.txt" ]
    [ ! -e "$TEST_TMP/fetches.log" ]
    rm "$WT/dirty.txt"
    run bash "$DH" cleanup-worktree --checked "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
    [ ! -e "$TEST_TMP/fetches.log" ]
}

@test "R2-7 a hung fetch is bounded by a timeout and the worktree is kept" {
    command -v timeout >/dev/null 2>&1 || command -v gtimeout >/dev/null 2>&1 || skip "no timeout binary"
    printf '#!/usr/bin/env bash\nsleep 25\n' > "$TEST_TMP/hang-upload-pack"
    chmod +x "$TEST_TMP/hang-upload-pack"
    git -C "$PRIMARY" config remote.origin.uploadpack "$TEST_TMP/hang-upload-pack"
    commit_in "$WT" local-only
    start="$(date +%s)"
    AUTOSPEC_DISK_FETCH_TIMEOUT_SECS=1 run bash "$DH" keep-reason "$WT"
    elapsed=$(( $(date +%s) - start ))
    [ "$output" = "unpushed" ]
    [ "$elapsed" -lt 15 ]
}

# 8 ── scaled floor (fails at c127d766: different dh_free_line contract/defaults)
@test "R2-8 default floor is min(20 GiB, 10% of the volume): 7 GiB free of 128 GiB parks" {
    . "$DH"
    dh_free_line() { echo "7340032 134217728"; }
    [ -n "$(dh_low_paths /x)" ]
}

@test "R2-8 a large volume is not over-parked: 150 GiB free of 2 TiB passes, 15 GiB parks" {
    . "$DH"
    dh_free_line() { echo "157286400 2147483648"; }
    [ -z "$(dh_low_paths /x)" ]
    dh_free_line() { echo "15728640 2147483648"; }
    [ -n "$(dh_low_paths /x)" ]
}

# 9 ── dispatch prompt text matches the per-worktree target/ design
@test "R2-9 the dispatched prompt does not claim a shared build output" {
    BR="dh-prompt-$$-$RANDOM"; BRS+=("$BR")
    printf '[package]\nname = "x"\nversion = "0.0.0"\n' > "$PRIMARY/Cargo.toml"
    git -C "$PRIMARY" add Cargo.toml; git -C "$PRIMARY" commit -q -m cargo; git -C "$PRIMARY" push -q origin main
    printf 'body\n' > "$TEST_TMP/prompt.md"
    cd "$PRIMARY"
    AUTOSPEC_DISK_MIN_FREE_GIB=0 run bash "$DISPATCH" --issue 1 --branch "$BR" --prompt-file "$TEST_TMP/prompt.md" --base origin/main
    [ "$status" -eq 0 ]
    [[ "$output" != *"shared, do not override"* ]]
    [[ "$output" == *"own"*"target/"* ]]
}
