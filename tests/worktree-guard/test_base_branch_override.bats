#!/usr/bin/env bats
# tests/worktree-guard/test_base_branch_override.bats — configurable base refs.
#
# Uses real local git remotes/worktrees. A tiny gh shim is used only for the
# default-branch fallback because the production path shells out to gh.

if [ -z "${BATS_VERSION:-}" ]; then
    exec bats "$0"
fi

ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd -P)"
GUARD="$ROOT/scripts/worktree-guard.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    unset AUTOSPEC_BASE_BRANCH

    ORIGIN="$TEST_TMP/origin.git"
    PRIMARY="$TEST_TMP/primary"
    git init -q --bare "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY"
}

teardown() {
    unset AUTOSPEC_BASE_BRANCH
    rm -rf "$TEST_TMP"
}

seed_branch() {
    local branch="$1"
    local content="$2"
    git -C "$PRIMARY" checkout -q --orphan "$branch"
    if ! git -C "$PRIMARY" rm -rf . >/dev/null 2>&1; then
        :
    fi
    printf '%s\n' "$content" > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add seed.txt
    git -C "$PRIMARY" commit -q -m "seed $branch"
    git -C "$PRIMARY" push -q -u origin "$branch"
}

install_gh_default_branch_shim() {
    local branch="$1"
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
    printf '%s\n' "${AUTOSPEC_TEST_DEFAULT_BRANCH}"
    exit 0
fi
echo "unexpected gh invocation: $*" >&2
exit 1
SH
    chmod +x "$TEST_TMP/bin/gh"
    export AUTOSPEC_TEST_DEFAULT_BRANCH="$branch"
    export PATH="$TEST_TMP/bin:$PATH"
}

@test "assert: AUTOSPEC_BASE_BRANCH=master_ai targets origin/master_ai" {
    seed_branch main main
    seed_branch master_ai master-ai
    wt="$TEST_TMP/wt-env"
    git -C "$PRIMARY" worktree add -q -b feat/env "$wt" origin/master_ai

    run bash -c "cd '$wt' && AUTOSPEC_BASE_BRANCH=master_ai bash '$GUARD' assert --strict-base"

    [ "$status" -eq 0 ]
}

@test "assert: --base overrides AUTOSPEC_BASE_BRANCH" {
    seed_branch main main
    seed_branch master_ai master-ai
    wt="$TEST_TMP/wt-cli"
    git -C "$PRIMARY" worktree add -q -b feat/cli "$wt" origin/main

    run bash -c "cd '$wt' && AUTOSPEC_BASE_BRANCH=master_ai bash '$GUARD' assert --base origin/main --strict-base"

    [ "$status" -eq 0 ]
}

@test "assert: plain branch names containing slashes still target origin/<branch>" {
    seed_branch main main
    seed_branch release/2026 release-2026
    wt="$TEST_TMP/wt-slash"
    git -C "$PRIMARY" worktree add -q -b feat/slash "$wt" origin/release/2026

    run bash -c "cd '$wt' && AUTOSPEC_BASE_BRANCH=release/2026 bash '$GUARD' assert --strict-base"

    [ "$status" -eq 0 ]
}

@test "assert: full remote refs are preserved" {
    seed_branch main main
    seed_branch master_ai master-ai
    wt="$TEST_TMP/wt-remote-ref"
    git -C "$PRIMARY" worktree add -q -b feat/remote-ref "$wt" origin/master_ai

    run bash -c "cd '$wt' && AUTOSPEC_BASE_BRANCH=origin/master_ai bash '$GUARD' assert --strict-base"

    [ "$status" -eq 0 ]
}

@test "create: falls back to gh default branch when origin/main is absent" {
    seed_branch master_ai master-ai
    install_gh_default_branch_shim master_ai
    wt="$TEST_TMP/wt-fallback"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/fallback --path '$wt'"

    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ "$(cat "$wt/seed.txt")" = "master-ai" ]
}

@test "create: .autospec/autospec.yml git.base_branch is honored when env is unset" {
    seed_branch main main
    seed_branch master_ai master-ai
    mkdir -p "$PRIMARY/.autospec"
    cat > "$PRIMARY/.autospec/autospec.yml" <<'YAML'
git:
  base_branch: master_ai
YAML
    wt="$TEST_TMP/wt-config"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/config --path '$wt'"

    [ "$status" -eq 0 ]
    [ "$(cat "$wt/seed.txt")" = "master-ai" ]
}

@test "create: default behavior still uses origin/main when present" {
    seed_branch main main
    seed_branch master_ai master-ai
    wt="$TEST_TMP/wt-default"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/default --path '$wt'"

    [ "$status" -eq 0 ]
    [ "$(cat "$wt/seed.txt")" = "main" ]
}
