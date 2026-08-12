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
    mkdir -p "$SOURCE_REPO/scripts"
    # Sized to sit over AUTOSPEC_MAX_FILE_LOC so that appending one line produces a
    # COMPLEXITY finding — these tests are about whether the issue argument reaches the
    # linter and suppresses that finding, not about the threshold itself. The count
    # tracks the limit: at 401 lines the fixture stopped tripping the rule when the
    # default moved from 400 to 600, and the tests silently passed on an empty result.
    printf '%s\n' '#!/usr/bin/env bash' > "$SOURCE_REPO/scripts/lint-implementation.sh"
    local line=1
    while [ "$line" -le 700 ]; do
        printf '# baseline fixture line %s\n' "$line" >> "$SOURCE_REPO/scripts/lint-implementation.sh"
        line=$((line + 1))
    done
    git -C "$SOURCE_REPO" add README.md scripts/lint-implementation.sh
    git -C "$SOURCE_REPO" commit -q -m "initial"
    git -C "$SOURCE_REPO" worktree add -q -b fix/2371-hook "$WORKTREE"
}

teardown() {
    rm -rf "$TEST_ROOT"
}

stage_complexity_violation() {
    local fixture="$WORKTREE/scripts/lint-implementation.sh"
    printf '%s\n' '# staged fixture line' >> "$fixture"
    git -C "$WORKTREE" add scripts/lint-implementation.sh
}

# Every case below asks for AUTOSPEC_COMPLEXITY_ENFORCE=1. This suite's subject is whether
# the installed hook FORWARDS --issue so the per-issue opt-out can apply, and observing that
# needs a violation that actually blocks: COMPLEXITY is advisory by default (design doc
# Fix 5), so without enforcement every assertion here holds whether or not the issue
# argument ever reached the linter. Exported rather than prefixed onto `run`, because the
# hook is a subprocess of `git commit` and only exported variables reach it.
enforce_complexity() {
    export AUTOSPEC_COMPLEXITY_ENFORCE=1
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
    enforce_complexity
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    stage_complexity_violation

    run git -C "$WORKTREE" commit -m "test numeric issue"
    [ "$status" -eq 0 ]
}

@test "installed hook passes the autonomous branch issue to staged lint" {
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    enforce_complexity
    git -C "$WORKTREE" checkout -q -b feat/autonomous-issue-2371
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    stage_complexity_violation

    run git -C "$WORKTREE" commit -m "test autonomous issue"
    [ "$status" -eq 0 ]
}

@test "installed hook rejects a noncanonical autonomous lookalike branch" {
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    enforce_complexity
    git -C "$WORKTREE" checkout -q -b fix/autonomous-issue-2371
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    stage_complexity_violation

    run git -C "$WORKTREE" commit -m "test autonomous lookalike"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q '^COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "installed hook omits issue arguments on a branch without a numeric issue segment" {
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    enforce_complexity
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

    run bash -c "cd '$WORKTREE' && AUTOSPEC_COMPLEXITY_ENFORCE=1 bash '$LINT_SCRIPT' --diff-file '$TEST_ROOT/complexity.diff' --issue 2371"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "staged mode loads issue skip directives before detectors run" {
    stage_complexity_violation

    run bash -c "cd '$WORKTREE' && AUTOSPEC_COMPLEXITY_ENFORCE=1 bash '$LINT_SCRIPT' --staged --issue 2371"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:COMPLEXITY:scripts/lint-implementation.sh:'
}

@test "failed issue lookup leaves a staged violation blocking" {
    stage_complexity_violation

    run bash -c "cd '$WORKTREE' && AUTOSPEC_COMPLEXITY_ENFORCE=1 bash '$LINT_SCRIPT' --staged --issue 999999999"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q '^COMPLEXITY:scripts/lint-implementation.sh:'
}
