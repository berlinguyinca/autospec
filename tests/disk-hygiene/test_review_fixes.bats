#!/usr/bin/env bats
# tests/disk-hygiene/test_review_fixes.bats — regression tests for the review
# findings on the disk-hygiene PR. Real git repos, real bare remotes, real
# (locked) worktrees and real directories in a per-test temp dir; the real
# scripts run. The only stand-in left: one `gh` stub for the "PR MERGED at
# exactly HEAD after a squash-merge" case (no local git fact equals it).

ROOT="${BATS_TEST_DIRNAME}/../.."
DH="$ROOT/scripts/disk-hygiene.sh"
DISPATCH="$ROOT/scripts/dispatch-implementer.sh"
GUARD="$ROOT/scripts/worktree-guard.sh"
GATE="$ROOT/scripts/explore-qa-gate.sh"
PROMPT_GEN="$ROOT/scripts/gen-implementer-prompt.sh"
LOOP_LIB="$ROOT/scripts/lib/autospec-loop.sh"

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
    printf '[package]\nname = "x"\nversion = "0.0.0"\n' > "$PRIMARY/Cargo.toml"
    echo seed > "$PRIMARY/seed.txt"
    echo "target/" > "$PRIMARY/.gitignore"
    git -C "$PRIMARY" add -A
    git -C "$PRIMARY" commit -q -m seed
    git -C "$PRIMARY" push -q -u origin main
    WT="$TEST_TMP/wt-a"
    git -C "$PRIMARY" worktree add -q -b feat/a "$WT" origin/main
    mkdir -p "$WT/target/debug"
    head -c 4096 /dev/zero > "$WT/target/debug/blob"
    BRS=()
}

teardown() {
    local b
    for b in "${BRS[@]:-}"; do
        [ -n "$b" ] && rm -rf "/tmp/wt-$b"
    done
    rm -rf "$TEST_TMP"
}

# ── Finding 2: guarded removal ───────────────────────────────────────────────

@test "F2 cleanup-worktree refuses a locked worktree (no rm -rf fallback)" {
    git -C "$PRIMARY" worktree lock "$WT"
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -f "$WT/target/debug/blob" ]
    [[ "$output" == *"locked"* ]]
    run bash "$DH" cleanup-worktree --force "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
}

@test "F2 cleanup-worktree refuses a subdirectory of a worktree" {
    run bash "$DH" cleanup-worktree --force "$WT/target"
    [ "$status" -eq 2 ]
    [ -f "$WT/target/debug/blob" ]
    [[ "$output" == *"not_worktree_root"* ]]
}

@test "F2 cleanup-worktree resolves a relative path against the caller, removing exactly that worktree" {
    cd "$TEST_TMP"
    run bash "$DH" cleanup-worktree "wt-a"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
    run git -C "$PRIMARY" worktree list --porcelain
    [[ "$output" != *"wt-a"* ]]
}

@test "F2 cleanup-worktree refuses a git checkout that is not a registered worktree" {
    git clone -q "$ORIGIN" "$TEST_TMP/stray" 2>/dev/null
    run bash "$DH" cleanup-worktree --force "$TEST_TMP/stray"
    [ "$status" -eq 2 ]
    [ -d "$TEST_TMP/stray/.git" ]
}

# ── Finding 5: "pushed" semantics ────────────────────────────────────────────

@test "F5 a branch deleted on the remote is NOT pushed (stale remote ref pruned)" {
    echo c > "$WT/c.txt"; git -C "$WT" add c.txt; git -C "$WT" commit -q -m c
    git -C "$WT" push -q -u origin feat/a
    git -C "$ORIGIN" branch -q -D feat/a
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
    [[ "$output" == *"unpushed"* ]]
}

@test "F5 work patch-equivalent to the default branch counts as pushed (squash/cherry-pick merge)" {
    echo c > "$WT/c.txt"; git -C "$WT" add c.txt; git -C "$WT" commit -q -m c
    sha="$(git -C "$WT" rev-parse HEAD)"
    git -C "$PRIMARY" cherry-pick "$sha" >/dev/null
    git -C "$PRIMARY" push -q origin main
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

@test "F5 HEAD reachable from another remote branch but not its own upstream is NOT pushed" {
    git -C "$PRIMARY" checkout -q -b other
    echo o > "$PRIMARY/o.txt"; git -C "$PRIMARY" add o.txt; git -C "$PRIMARY" commit -q -m o
    git -C "$PRIMARY" push -q origin other
    git -C "$PRIMARY" checkout -q main
    git -C "$WT" push -q -u origin feat/a
    git -C "$WT" fetch -q origin
    git -C "$WT" reset -q --hard origin/other
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
}

@test "F5 a PR MERGED at exactly HEAD counts as pushed even with the branch deleted" {
    echo c > "$WT/c.txt"; git -C "$WT" add c.txt; git -C "$WT" commit -q -m c
    echo c2 >> "$WT/c.txt"; git -C "$WT" commit -q -am c2   # not patch-equivalent anywhere
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<EOS
#!/usr/bin/env bash
# GitHub API stand-in: the PR for feat/a merged at exactly this worktree's HEAD.
git rev-parse HEAD
EOS
    chmod +x "$TEST_TMP/bin/gh"
    PATH="$TEST_TMP/bin:$PATH" run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

# ── Finding 6: preflight thresholds and paths ────────────────────────────────

@test "F6 preflight checks CARGO_TARGET_DIR and HOME as well as the given path" {
    mkdir -p "$TEST_TMP/ct"
    CARGO_TARGET_DIR="$TEST_TMP/ct" AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 \
        run bash "$DH" preflight --path "$WT"
    [ "$status" -eq 75 ]
    [[ "$output" == *"$TEST_TMP/ct "* ]]
    [[ "$output" == *"$HOME "* ]]
}

@test "F6 a path is low only when below BOTH the percentage and the absolute floor" {
    AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=0 run bash "$DH" preflight --path "$WT"
    [ "$status" -eq 0 ]
    AUTOSPEC_DISK_MIN_FREE_PCT=0 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 run bash "$DH" preflight --path "$WT"
    [ "$status" -eq 0 ]
}

# ── Finding 10: no shared cargo target ───────────────────────────────────────

@test "F10 env never exports a shared CARGO_TARGET_DIR, even with the old opt-in variables" {
    AUTOSPEC_CARGO_TARGET_ROOT="$TEST_TMP/targets" AUTOSPEC_SHARED_TARGET=on run bash "$DH" env "$WT"
    [ "$status" -eq 0 ]
    [[ "$output" != *"CARGO_TARGET_DIR"* ]]
    [[ "$output" == *"CARGO_INCREMENTAL=0"* ]]
}

# ── Finding 11: env file on reuse; prompt creates worktrees via the guard ────

@test "F11 worktree-guard writes .autospec-env.sh when REUSING a clean worktree" {
    cd "$PRIMARY"
    run bash "$GUARD" create --branch feat/a --path "$WT" --base origin/main
    [ "$status" -eq 0 ]
    grep -q "CARGO_INCREMENTAL=0" "$WT/.autospec-env.sh"
    [ -z "$(git -C "$WT" status --porcelain)" ]
}

@test "F11 the generated prompt's worktree setup block yields .autospec-env.sh when run" {
    printf '# Issue\n\nbody\n' > "$TEST_TMP/issue.md"
    run bash "$PROMPT_GEN" --issue-body "$TEST_TMP/issue.md" --branch feat/prompted
    [ "$status" -eq 0 ]
    [[ "$output" != *"git worktree add"* ]]
    printf '%s\n' "$output" | awk '/^### Worktree setup/{f=1;next} f&&/^```bash/{b=1;next} b&&/^```/{exit} b' > "$TEST_TMP/setup.sh"
    sed -i.bak "s#cd /path/to/autospec#cd '$PRIMARY'#" "$TEST_TMP/setup.sh"
    grep -q 'worktree-guard.sh" create' "$TEST_TMP/setup.sh"
    BR="feat-prompted"
    BRS+=("$BR")
    AUTOSPEC_SCRIPTS_DIR="$ROOT/scripts" run bash -c "set -e; . '$TEST_TMP/setup.sh'; test -f .autospec-env.sh; pwd"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/tmp/wt-feat-prompted" ]]
    git -C "$PRIMARY" worktree remove --force /tmp/wt-feat-prompted
}

# ── Finding 8: ownership across a dispatcher race ────────────────────────────

@test "F8 a dispatcher that merely reuses a worktree created by a racing winner never deletes it" {
    BR="dh-race-$$-$RANDOM"; BRS+=("$BR")
    printf 'p\n' > "$TEST_TMP/prompt.md"
    # A real barrier, not a fake: upload-pack for `origin` waits until released,
    # so the winner can create the worktree between the loser's start and its
    # worktree-guard create.
    cat > "$TEST_TMP/gate-upload-pack" <<EOS
#!/usr/bin/env bash
touch "$TEST_TMP/fetching"
while [ ! -e "$TEST_TMP/go" ]; do sleep 0.1; done
exec "\$(git --exec-path)/git-upload-pack" "\$@"
EOS
    chmod +x "$TEST_TMP/gate-upload-pack"
    git -C "$PRIMARY" config remote.origin.uploadpack "$TEST_TMP/gate-upload-pack"
    (
        cd "$PRIMARY"
        rc=0
        AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 \
            bash "$DISPATCH" --issue 1 --branch "$BR" --prompt-file "$TEST_TMP/prompt.md" \
            --base origin/main > "$TEST_TMP/loser.out" 2>&1 || rc=$?
        echo "$rc" > "$TEST_TMP/loser.rc"
    ) &
    loser=$!
    for _ in $(seq 1 100); do [ -e "$TEST_TMP/fetching" ] && break; sleep 0.1; done
    [ -e "$TEST_TMP/fetching" ]
    git -C "$PRIMARY" -c remote.origin.uploadpack=git-upload-pack worktree add -q -b "$BR" "/tmp/wt-$BR" origin/main
    touch "$TEST_TMP/go"
    wait "$loser"
    [ "$(cat "$TEST_TMP/loser.rc")" -eq 75 ]
    [ -d "/tmp/wt-$BR" ]
}

@test "F8 a dispatcher that really created the worktree still removes it when it aborts" {
    BR="dh-own-$$-$RANDOM"; BRS+=("$BR")
    printf 'p\n' > "$TEST_TMP/prompt.md"
    cd "$PRIMARY"
    AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 \
        run bash "$DISPATCH" --issue 1 --branch "$BR" --prompt-file "$TEST_TMP/prompt.md" --base origin/main
    [ "$status" -eq 75 ]
    [ ! -e "/tmp/wt-$BR" ]
}

# ── Finding 9: --cleanup of a leftover non-git directory ─────────────────────

@test "F9 dispatch-implementer --cleanup removes a leftover plain directory at /tmp/wt-<branch>" {
    BR="dh-plain-$$-$RANDOM"; BRS+=("$BR")
    mkdir -p "/tmp/wt-$BR/sub"; echo x > "/tmp/wt-$BR/sub/f"
    cd "$PRIMARY"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 0 ]
    [ ! -e "/tmp/wt-$BR" ]
}

@test "F9 dispatch-implementer --cleanup never removes an unregistered git checkout" {
    BR="dh-repo-$$-$RANDOM"; BRS+=("$BR")
    git init -q "/tmp/wt-$BR"
    cd "$PRIMARY"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 2 ]
    [ -d "/tmp/wt-$BR/.git" ]
}

# ── Finding 1: explore-qa-gate never deletes a worktree it did not create ────

@test "F1 explore-qa-gate leaves a refused dirty worktree at WT_PATH untouched" {
    cd "$PRIMARY"
    mkdir -p .autospec
    : > .autospec/test.yml
    git add .autospec/test.yml >/dev/null 2>&1 || true
    HEAD_SHA="$(git rev-parse HEAD)"
    printf '{"branch":"main","slug":"d","base":"main","head_sha":"%s","created_at":"2026-01-01T00:00:00Z"}\n' "$HEAD_SHA" > .autospec/explore-mode.json
    DIRTY="$TEST_TMP/wt-dirty"
    git worktree add -q -b someone-elses "$DIRTY" "$HEAD_SHA"
    echo precious > "$DIRTY/unsaved.txt"
    AUTOSPEC_QA_GATE_WORKTREE="$DIRTY" AUTOSPEC_STATE_DIR="$TEST_TMP/state" run bash "$GATE"
    [ "$status" -eq 1 ]
    [ -f "$DIRTY/unsaved.txt" ]
    [ "$(cat "$DIRTY/unsaved.txt")" = precious ]
}

# ── Finding 7: a disk-parked conductor still heartbeats, polls control, counts ─

@test "F7 a disk-parked conductor cycle heartbeats, writes state and honours CONDUCTOR_MAX_CYCLES (real helper scripts)" {
    # The REAL control-channel / resilience / runtime-config / disk-hygiene
    # scripts from the repo, copied into a sandbox scripts dir (no fakes). No gh
    # on the path to a real host: the control channel polls an unreachable host
    # and must fail open.
    FS="$TEST_TMP/scripts"; mkdir -p "$FS"
    cp "$ROOT/scripts/disk-hygiene.sh" "$ROOT/scripts/autospec-runtime-config.sh" \
       "$ROOT/scripts/autonomous-control-channel.sh" "$ROOT/scripts/autonomous-resilience.sh" "$FS/"
    export HOME="$TEST_TMP"
    GH_HOST=unreachable.invalid GH_TOKEN=x AUTOSPEC_CONFIG_FILE="$TEST_TMP/none.yml" \
        run perl -e 'alarm 90; exec @ARGV' bash -c "
        . '$LOOP_LIB'
        CONDUCTOR_SCRIPTS_DIR='$FS' CONDUCTOR_REPO=o/n CONDUCTOR_MAX_CYCLES=2 \
        CONDUCTOR_POLL_INTERVAL=0 AUTOSPEC_DISK_PARK_SLEEP_SECS=0 CONDUCTOR_NO_DIGEST=1 \
        AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 \
        autospec_conductor_run"
    [ "$status" -ne 142 ]
    [[ "$output" == *"cycle 1 parked: disk low"* ]]
    [[ "$output" == *"cycle 2 parked: disk low"* ]]
    [[ "$output" == *"STATUS:parked:disk-low:cycle-2"* ]]
    [[ "$output" == *"stopped: max-cycles-reached (cycles=2)"* ]]
    [[ "$output" == *"disk_low_parked"* ]]
}
