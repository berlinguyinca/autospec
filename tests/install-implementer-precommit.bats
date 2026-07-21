#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    INSTALL_SCRIPT="$REPO_ROOT/scripts/install-implementer-precommit.sh"
    LINT_SCRIPT="$REPO_ROOT/scripts/lint-implementation.sh"
    TEST_ROOT="$(mktemp -d)"
    SOURCE_REPO="$TEST_ROOT/source"
    WORKTREE="$TEST_ROOT/worktree"

    git init -q "$SOURCE_REPO"
    git -C "$SOURCE_REPO" config user.email "test@example.com"
    git -C "$SOURCE_REPO" config user.name "Test User"
    git -C "$SOURCE_REPO" remote add origin https://github.com/berlinguyinca/autospec.git
    touch "$SOURCE_REPO/README.md"
    git -C "$SOURCE_REPO" add README.md
    git -C "$SOURCE_REPO" commit -q -m "initial"
    git -C "$SOURCE_REPO" worktree add -q -b fix/2371-hook "$WORKTREE"
}

teardown() {
    rm -rf "$TEST_ROOT"
}

stage_complexity_violation() {
    mkdir -p "$WORKTREE/scripts"
    local fixture="$WORKTREE/scripts/lint-implementation.sh"
    printf '%s\n' '#!/usr/bin/env bash' > "$fixture"
    local line=1
    while [ "$line" -le 401 ]; do
        printf '# fixture line %s\n' "$line" >> "$fixture"
        line=$((line + 1))
    done
    git -C "$WORKTREE" add scripts/lint-implementation.sh
}

@test "integration suite does not replace gh or lint-implementation" {
    run grep -nE '^make_(recording_linter|issue_gh)\(\)' "$BATS_TEST_FILENAME"
    [ "$status" -eq 1 ]
}

@test "installer writes an executable hook to Git's resolved linked-worktree path" {
    run bash "$INSTALL_SCRIPT" "$WORKTREE"
    [ "$status" -eq 0 ]

    hook_path="$(git -C "$WORKTREE" rev-parse --git-path hooks/pre-commit)"
    [ -x "$hook_path" ]
}

@test "installed hook passes the numeric branch issue to staged lint" {
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    stage_complexity_violation

    run git -C "$WORKTREE" commit -m "test numeric issue"
    [ "$status" -eq 0 ]
}

@test "installed hook omits issue arguments on a branch without a numeric issue segment" {
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    git -C "$WORKTREE" checkout -q -b fix/hook-without-issue
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    stage_complexity_violation

    run git -C "$WORKTREE" commit -m "test missing issue"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q '^COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "diff-file mode loads issue skip directives before detectors run" {
    stage_complexity_violation
    git -C "$WORKTREE" diff --cached --output="$TEST_ROOT/complexity.diff"

    run bash -c "cd '$WORKTREE' && bash '$LINT_SCRIPT' --diff-file '$TEST_ROOT/complexity.diff' --issue 2371"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "staged mode loads issue skip directives before detectors run" {
    stage_complexity_violation

    run bash -c "cd '$WORKTREE' && bash '$LINT_SCRIPT' --staged --issue 2371"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "failed issue lookup leaves a staged violation blocking" {
    stage_complexity_violation

    run bash -c "cd '$WORKTREE' && bash '$LINT_SCRIPT' --staged --issue 999999999"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q '^COMPLEXITY:scripts/lint-implementation.sh:'
}
