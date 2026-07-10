#!/usr/bin/env bats
# tests/autonomous/test_integration_branch.bats — autonomous integration branch lifecycle.

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
    export AUTOSPEC_CONFIG_FILE="$TMP/autospec.yml"
    export PATH="$FAKE_BIN:$PATH"

    touch "$GIT_CALLS" "$GH_CALLS"
    printf 'parent_sha=parent111\nintegration_sha=integration222\nbranch_exists=0\nmerge_conflict=0\ndiff_lines=12\nage_epoch=1700000000\n' > "$GIT_STATE"

    cat > "$FAKE_BIN/git" <<'EOF'
#!/usr/bin/env bash
set -eu
state_get() {
    grep "^$1=" "$GIT_STATE" | tail -1 | cut -d= -f2-
}
printf '%s\n' "$*" >> "$GIT_CALLS"
case "${1:-}" in
    rev-parse)
        if [ "${2:-}" = "--show-toplevel" ]; then
            printf '%s\n' "$GIT_ROOT"
        elif [ "${2:-}" = "origin/main" ]; then
            state_get parent_sha
        elif [ "${2:-}" = "autospec/autonomous-main" ]; then
            state_get integration_sha
        else
            printf 'sha-%s\n' "${2:-HEAD}"
        fi
        ;;
    fetch)
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
    branch)
        exit 0
        ;;
    push)
        exit 0
        ;;
    checkout)
        exit 0
        ;;
    merge)
        if [ "$(state_get merge_conflict)" = "1" ]; then exit 1; fi
        exit 0
        ;;
    merge-base)
        state_get integration_sha
        ;;
    log)
        state_get age_epoch
        ;;
    diff)
        i=0
        lines="$(state_get diff_lines)"
        while [ "$i" -lt "$lines" ]; do
            printf 'diff line %s\n' "$i"
            i=$((i + 1))
        done
        ;;
    *)
        exit 0
        ;;
esac
EOF
    chmod +x "$FAKE_BIN/git"

    cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$GH_CALLS"
case "${1:-} ${2:-}" in
    "pr list")
        printf '[{"number":77,"state":"OPEN"}]\n'
        ;;
    *)
        printf '{}\n'
        ;;
esac
EOF
    chmod +x "$FAKE_BIN/gh"
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

@test "sync aborts conflicted parent merge and exits 65" {
    printf 'branch_exists=1\nmerge_conflict=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" sync --parent main

    [ "$status" -eq 65 ]
    grep -q 'checkout autospec/autonomous-main' "$GIT_CALLS"
    grep -q 'merge --no-edit origin/main' "$GIT_CALLS"
    grep -q 'merge --abort' "$GIT_CALLS"
}

@test "reset recreates integration branch from new parent tip" {
    run bash "$SCRIPT" reset --parent main

    [ "$status" -eq 0 ]
    grep -q 'branch -f autospec/autonomous-main origin/main' "$GIT_CALLS"
    grep -q 'push -u origin autospec/autonomous-main' "$GIT_CALLS"
}

@test "status emits rollup pr, accumulated count, age days, and diff lines" {
    printf 'branch_exists=1\n' >> "$GIT_STATE"

    run bash "$SCRIPT" status --parent main

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.branch')" = "autospec/autonomous-main" ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.number')" = "77" ]
    [ "$(printf '%s' "$output" | jq -r '.rollup_pr.state')" = "OPEN" ]
    [ "$(printf '%s' "$output" | jq -r '.accumulated_pr_count')" = "1" ]
    [ "$(printf '%s' "$output" | jq -r '.diff_lines')" = "12" ]
    [ "$(printf '%s' "$output" | jq -r '.age_days >= 0')" = "true" ]
    grep -q 'pr list --repo berlinguyinca/autospec --head autospec/autonomous-main --base main --state all --json number,state' "$GH_CALLS"
}
