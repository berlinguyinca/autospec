#!/usr/bin/env bats
# tests/autonomous/test_integration_branch.bats — autonomous integration branch lifecycle.

write_fake_git() {
    write_fake_git_header > "$FAKE_BIN/git"
    write_fake_git_refs >> "$FAKE_BIN/git"
    write_fake_git_history >> "$FAKE_BIN/git"
    write_fake_git_footer >> "$FAKE_BIN/git"
    chmod +x "$FAKE_BIN/git"
}

write_fake_git_header() {
    cat <<'EOF'
#!/usr/bin/env bash
set -eu
state_get() { grep "^$1=" "$GIT_STATE" | tail -1 | cut -d= -f2-; }
printf '%s\n' "$*" >> "$GIT_CALLS"
case "${1:-}" in
    rev-parse)
        if [ "${2:-}" = "--show-toplevel" ]; then
            printf '%s\n' "$GIT_ROOT"
        elif [ "${2:-}" = "origin/main" ]; then
            state_get parent_sha
        elif [ "${2:-}" = "autospec/autonomous-main" ] || [ "${2:-}" = "autospec/autonomous-feat/x" ]; then
            state_get integration_sha
        else
            printf 'sha-%s\n' "${2:-HEAD}"
        fi
        ;;
EOF
}

write_fake_git_refs() {
    cat <<'EOF'
    fetch)
        if [ "$(state_get fetch_failure)" = "1" ]; then exit 9; fi
        exit 0
        ;;
    branch)
        exit 0
        ;;
    push)
        if [ "$(state_get push_failure)" = "1" ]; then exit 8; fi
        exit 0
        ;;
    checkout|worktree)
        exit 0
        ;;
    show-ref)
        if [ "$(state_get branch_exists)" = "1" ]; then exit 0; fi
        exit 1
        ;;
    ls-remote)
        if [ "$(state_get branch_exists)" = "1" ]; then
            printf '%s\trefs/heads/%s\n' "$(state_get integration_sha)" "${@: -1}"
            exit 0
        fi
        exit 2
        ;;
EOF
}

write_fake_git_history() {
    cat <<'EOF'
    merge) if [ "$(state_get merge_conflict)" = "1" ]; then exit 1; fi ;;
    merge-base) state_get integration_sha ;;
    rev-list)
        if [ "$(state_get revlist_failure)" = "1" ]; then exit 6; fi
        state_get merge_pr_count
        ;;
    log)
        if [ "$(state_get log_failure)" = "1" ]; then exit 3; fi
        state_get age_epoch
        ;;
    diff)
        if [ "$(state_get diff_failure)" = "1" ]; then exit 4; fi
        added="$(state_get diff_added)"
        deleted="$(state_get diff_deleted)"
        printf '%s\t%s\tfile.txt\n' "$added" "$deleted"
        ;;
    config)
        printf 'https://github.com/berlinguyinca/autospec.git\n'
        ;;
EOF
}

write_fake_git_footer() {
    cat <<'EOF'
    *) exit 0 ;;
esac
EOF
}

write_fake_gh() {
    cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$GH_CALLS"
if [ "${GH_FAILURE:-0}" = "1" ]; then
    exit 5
fi
case "${1:-} ${2:-}" in
    "pr list")
        case " $* " in
            *" --head autospec/autonomous-main --base main "*)
                cat "$GH_ROLLUP_JSON"
                ;;
            *)
                printf '[]\n'
                ;;
        esac
        ;;
    *)
        printf '{}\n'
        ;;
esac
EOF
    chmod +x "$FAKE_BIN/gh"
}

write_fake_state() {
    printf 'parent_sha=parent111\nintegration_sha=integration222\nbranch_exists=0\nmerge_conflict=0\ndiff_added=8\ndiff_deleted=4\nage_epoch=1700000000\nlog_failure=0\ndiff_failure=0\nfetch_failure=0\npush_failure=0\nrevlist_failure=0\nmerge_pr_count=3\ncurrent_branch=main\n' > "$GIT_STATE"
    printf '[{"number":77,"state":"OPEN"}]\n' > "$GH_ROLLUP_JSON"
}

setup_file() {
    REAL_GIT_BIN="$(command -v git)"
    export REAL_GIT_BIN
}

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-integration-branch.sh"
    TMP="$(mktemp -d -t integration-branch.XXXXXX)"
    FAKE_BIN="$TMP/bin"
    mkdir -p "$FAKE_BIN" "$TMP/repo"

    export GIT_CALLS="$TMP/git-calls.log"
    export GH_CALLS="$TMP/gh-calls.log"
    export GIT_STATE="$TMP/git-state"
    export GIT_ROOT="$TMP/repo"
    export GH_ROLLUP_JSON="$TMP/gh-rollup.json"
    export GH_FAILURE=0
    export AUTOSPEC_CONFIG_FILE="$TMP/autospec.yml"
    export PATH="$FAKE_BIN:$PATH"

    touch "$GIT_CALLS" "$GH_CALLS"
    write_fake_state
    write_fake_git
    write_fake_gh
}

teardown() {
    rm -rf "$TMP"
}

@test "ensure creates missing integration branch from parent tip and writes mode file" {
    run bash "$SCRIPT" ensure --parent main --repo berlinguyinca/autospec

    [ "$status" -eq 0 ]
    grep -q 'branch autospec/autonomous-main origin/main' "$GIT_CALLS"
    grep -q 'push -u origin autospec/autonomous-main' "$GIT_CALLS"
    [ -f "$GIT_ROOT/.autospec/explore-mode.json" ]
    [ "$(jq -r '.kind' "$GIT_ROOT/.autospec/explore-mode.json")" = "integration" ]
    [ "$(jq -r '.branch' "$GIT_ROOT/.autospec/explore-mode.json")" = "autospec/autonomous-main" ]
    [ "$(jq -r '.slug' "$GIT_ROOT/.autospec/explore-mode.json")" = "berlinguyinca/autospec" ]
    [ "$(jq -r '.base' "$GIT_ROOT/.autospec/explore-mode.json")" = "main" ]
    [ "$(jq -r '.head_sha' "$GIT_ROOT/.autospec/explore-mode.json")" = "integration222" ]
}

@test "ensure reuses existing integration branch and honors configured prefix" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"
    cat > "$AUTOSPEC_CONFIG_FILE" <<'YAML'
autonomous:
  self_originated:
    integration_branch_prefix: team/integration-
YAML

    run bash "$SCRIPT" ensure --parent dev --repo example/repo

    [ "$status" -eq 0 ]
    ! grep -q '^branch ' "$GIT_CALLS"
    ! grep -q '^push ' "$GIT_CALLS"
    [ "$(jq -r '.branch' "$GIT_ROOT/.autospec/explore-mode.json")" = "team/integration-dev" ]
    [ "$(jq -r '.base' "$GIT_ROOT/.autospec/explore-mode.json")" = "dev" ]
}

@test "ensure accepts nested parent branch names" {
    run bash "$SCRIPT" ensure --parent feat/x --repo example/repo

    [ "$status" -eq 0 ]
    [ "$(jq -r '.branch' "$GIT_ROOT/.autospec/explore-mode.json")" = "autospec/autonomous-feat/x" ]
    [ "$(jq -r '.base' "$GIT_ROOT/.autospec/explore-mode.json")" = "feat/x" ]
}

@test "ensure refuses to clobber a conflicting explore-mode.json" {
    mkdir -p "$GIT_ROOT/.autospec"
    printf '{"branch":"autospec/explore/2026-01-01-abc","slug":"x/y","base":"main","head_sha":"abc","kind":"explore"}\n' \
        > "$GIT_ROOT/.autospec/explore-mode.json"

    run bash "$SCRIPT" ensure --parent main --repo berlinguyinca/autospec

    [ "$status" -eq 6 ]
    [[ "$output" == *"code_health:integration_mode_conflict"* ]]
    [ "$(jq -r '.kind' "$GIT_ROOT/.autospec/explore-mode.json")" = "explore" ]
}

@test "ensure is idempotent when the mode file already records this integration branch" {
    mkdir -p "$GIT_ROOT/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"berlinguyinca/autospec","base":"main","head_sha":"old","kind":"integration"}\n' \
        > "$GIT_ROOT/.autospec/explore-mode.json"

    run bash "$SCRIPT" ensure --parent main --repo berlinguyinca/autospec

    [ "$status" -eq 0 ]
    [ "$(jq -r '.head_sha' "$GIT_ROOT/.autospec/explore-mode.json")" = "integration222" ]
}

@test "ensure --takeover overwrites a conflicting explore-mode.json" {
    mkdir -p "$GIT_ROOT/.autospec"
    printf '{"branch":"autospec/explore/2026-01-01-abc","slug":"x/y","base":"main","head_sha":"abc","kind":"explore"}\n' \
        > "$GIT_ROOT/.autospec/explore-mode.json"

    run bash "$SCRIPT" ensure --parent main --repo berlinguyinca/autospec --takeover

    [ "$status" -eq 0 ]
    [ "$(jq -r '.kind' "$GIT_ROOT/.autospec/explore-mode.json")" = "integration" ]
    [ "$(jq -r '.branch' "$GIT_ROOT/.autospec/explore-mode.json")" = "autospec/autonomous-main" ]
}

@test "sync happy path merges parent tip in a temp worktree and pushes the integration branch" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" sync --parent main --repo example/repo

    [ "$status" -eq 0 ]
    grep -q 'worktree add --detach' "$GIT_CALLS"
    grep -q 'merge --no-edit origin/main' "$GIT_CALLS"
    grep -q 'push origin HEAD:autospec/autonomous-main' "$GIT_CALLS"
    ! grep -q '^checkout autospec/autonomous-main$' "$GIT_CALLS"
}

@test "sync aborts conflicted parent merge and exits 65" {
    printf 'branch_exists=1\nmerge_conflict=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" sync --parent main

    [ "$status" -eq 65 ]
    grep -q 'worktree add --detach' "$GIT_CALLS"
    grep -q 'merge --no-edit origin/main' "$GIT_CALLS"
    grep -q 'merge --abort' "$GIT_CALLS"
    ! grep -q '^push ' "$GIT_CALLS"
}

@test "ensure fails when parent fetch fails before branch creation" {
    printf 'fetch_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" ensure --parent main --repo berlinguyinca/autospec

    [ "$status" -ne 0 ]
    [[ "$output" == *"autonomous_integration_parent_fetch_failed"* ]]
    ! grep -q '^branch ' "$GIT_CALLS"
    ! grep -q '^push ' "$GIT_CALLS"
}

@test "sync fails when parent fetch fails before merge or push" {
    printf 'branch_exists=1\nfetch_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" sync --parent main

    [ "$status" -ne 0 ]
    [[ "$output" == *"autonomous_integration_parent_fetch_failed"* ]]
    ! grep -q '^checkout ' "$GIT_CALLS"
    ! grep -q '^merge ' "$GIT_CALLS"
    ! grep -q '^push ' "$GIT_CALLS"
}

@test "sync fails closed when the push fails" {
    printf 'branch_exists=1\npush_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" sync --parent main

    [ "$status" -ne 0 ]
    grep -q 'merge --no-edit origin/main' "$GIT_CALLS"
}

@test "reset recreates integration branch from new parent tip when no rollup pr is open" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"
    printf '[]\n' > "$GH_ROLLUP_JSON"

    run bash "$SCRIPT" reset --parent main --repo example/repo

    [ "$status" -eq 0 ]
    grep -q 'worktree add --detach' "$GIT_CALLS"
    grep -q 'push origin :autospec/autonomous-main' "$GIT_CALLS"
    grep -q 'push origin HEAD:autospec/autonomous-main' "$GIT_CALLS"
}

@test "reset refuses when the rollup pr is still open" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"
    printf '[{"number":77,"state":"OPEN"}]\n' > "$GH_ROLLUP_JSON"

    run bash "$SCRIPT" reset --parent main --repo berlinguyinca/autospec

    [ "$status" -ne 0 ]
    [[ "$output" == *"code_health:integration_reset_rollup_open"* ]]
    ! grep -q '^push ' "$GIT_CALLS"
    ! grep -q 'worktree add' "$GIT_CALLS"
}

@test "reset fails when parent fetch fails before branch reset" {
    printf 'fetch_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" reset --parent main

    [ "$status" -ne 0 ]
    [[ "$output" == *"autonomous_integration_parent_fetch_failed"* ]]
    ! grep -q '^push ' "$GIT_CALLS"
    ! grep -q 'worktree add' "$GIT_CALLS"
}

@test "status emits rollup pr, current-cycle merged pr count, age days, and diff lines" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" status --parent main

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.branch')" = "autospec/autonomous-main" ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.number')" = "77" ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.state')" = "OPEN" ]
    [ "$(printf '%s' "$output" | jq -r '.accumulated_pr_count')" = "3" ]
    [ "$(printf '%s' "$output" | jq -r '.diff_lines')" = "12" ]
    [ "$(printf '%s' "$output" | jq -r '.age_days >= 0')" = "true" ]
    grep -q 'pr list --repo berlinguyinca/autospec --head autospec/autonomous-main --base main --state open --json number,state' "$GH_CALLS"
    grep -q 'rev-list --count --merges origin/main..autospec/autonomous-main' "$GIT_CALLS"
}

@test "status succeeds with explicit empty rollup pr when no rollup exists" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"
    printf '[]\n' > "$GH_ROLLUP_JSON"

    run bash "$SCRIPT" status --parent main

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.number')" = "null" ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.state')" = "null" ]
    [ "$(printf '%s' "$output" | jq -r '.accumulated_pr_count')" = "3" ]
}

@test "status fails visibly when gh query fails" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"
    export GH_FAILURE=1

    run bash "$SCRIPT" status --parent main

    [ "$status" -ne 0 ]
    [[ "$output" == *"status probe failed"* ]]
}

@test "status fails visibly when git log fails" {
    printf 'branch_exists=1\nlog_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" status --parent main

    [ "$status" -ne 0 ]
    [[ "$output" == *"status probe failed"* ]]
}

@test "status fails visibly when git diff fails" {
    printf 'branch_exists=1\ndiff_failure=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" status --parent main

    [ "$status" -ne 0 ]
    [[ "$output" == *"status probe failed"* ]]
}

@test "--parent with a missing value dies instead of silently defaulting" {
    run bash "$SCRIPT" ensure --parent

    [ "$status" -eq 2 ]
    [[ "$output" == *"--parent requires a value"* ]]
}

@test "sync leaves the caller checkout's HEAD and branch untouched (real git)" {
    real_root="$TMP/realrepo"
    origin_git="$TMP/origin.git"
    mkdir -p "$real_root"
    "$REAL_GIT_BIN" init -q -b main "$real_root"
    ( cd "$real_root" && "$REAL_GIT_BIN" commit -q --allow-empty -m init )
    "$REAL_GIT_BIN" init -q --bare "$origin_git"
    ( cd "$real_root" && "$REAL_GIT_BIN" remote add origin "$origin_git" )
    ( cd "$real_root" && "$REAL_GIT_BIN" push -q origin main )
    ( cd "$real_root" && "$REAL_GIT_BIN" branch autospec/autonomous-main main )
    ( cd "$real_root" && "$REAL_GIT_BIN" push -q origin autospec/autonomous-main )
    ( cd "$real_root" && "$REAL_GIT_BIN" commit -q --allow-empty -m parent-advance )
    ( cd "$real_root" && "$REAL_GIT_BIN" push -q origin main )
    ( cd "$real_root" && "$REAL_GIT_BIN" checkout -q main )

    head_before="$(cd "$real_root" && "$REAL_GIT_BIN" rev-parse HEAD)"
    branch_before="$(cd "$real_root" && "$REAL_GIT_BIN" branch --show-current)"

    real_git_dir="$(dirname "$REAL_GIT_BIN")"
    PATH="$real_git_dir:$FAKE_BIN:$PATH" run bash -c \
        "cd '$real_root' && bash '$SCRIPT' sync --parent main --repo example/repo"

    [ "$status" -eq 0 ]

    head_after="$(cd "$real_root" && "$REAL_GIT_BIN" rev-parse HEAD)"
    branch_after="$(cd "$real_root" && "$REAL_GIT_BIN" branch --show-current)"

    [ "$head_before" = "$head_after" ]
    [ "$branch_before" = "$branch_after" ]

    remote_integration_sha="$(cd "$real_root" && "$REAL_GIT_BIN" ls-remote origin autospec/autonomous-main | cut -f1)"
    remote_main_sha="$(cd "$real_root" && "$REAL_GIT_BIN" ls-remote origin main | cut -f1)"
    [ "$remote_integration_sha" = "$remote_main_sha" ]
}
