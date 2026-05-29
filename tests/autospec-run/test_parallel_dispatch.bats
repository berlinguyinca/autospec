#!/usr/bin/env bats
# tests/autospec-run/test_parallel_dispatch.bats — parallel implementer
# worktree isolation contract (issue #690).
#
# Fixtures (per spec):
#   a. helper creates worktree at /tmp/wt-<branch>
#   b. prompt pre-pend includes worktree path
#   c. helper removes worktree on completion (--cleanup)
#   d. two parallel invocations don't collide (distinct worktrees, distinct branches)

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    HELPER="$REPO_ROOT/scripts/dispatch-implementer.sh"
    TMPDIR_BATS="$(mktemp -d)"
    PROMPT_FILE="$TMPDIR_BATS/prompt.md"
    printf 'IMPLEMENTER_PROMPT_BODY\n' > "$PROMPT_FILE"
    BRANCH_A="test-690-fixture-a-$$"
    BRANCH_B="test-690-fixture-b-$$"
}

teardown() {
    bash "$HELPER" --issue 690 --branch "$BRANCH_A" --cleanup 2>/dev/null || true
    bash "$HELPER" --issue 690 --branch "$BRANCH_B" --cleanup 2>/dev/null || true
    rm -rf "$TMPDIR_BATS"
}

@test "fixture a: helper creates worktree at /tmp/wt-<branch>" {
    run bash "$HELPER" --issue 690 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE"
    [ "$status" -eq 0 ]
    [ -d "/tmp/wt-$BRANCH_A" ]
}

@test "fixture b: prompt pre-pend includes worktree path" {
    run bash "$HELPER" --issue 690 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/tmp/wt-$BRANCH_A"* ]]
    [[ "$output" == *"IMPLEMENTER_PROMPT_BODY"* ]]
    [[ "$output" == *"Workdir:"* ]]
}

@test "fixture c: helper removes worktree on --cleanup" {
    bash "$HELPER" --issue 690 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE" >/dev/null
    [ -d "/tmp/wt-$BRANCH_A" ]
    run bash "$HELPER" --issue 690 --branch "$BRANCH_A" --cleanup
    [ "$status" -eq 0 ]
    [ ! -d "/tmp/wt-$BRANCH_A" ]
}

@test "fixture d: two parallel invocations produce distinct worktrees" {
    bash "$HELPER" --issue 690 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE" >/dev/null
    bash "$HELPER" --issue 691 --branch "$BRANCH_B" --prompt-file "$PROMPT_FILE" >/dev/null
    [ -d "/tmp/wt-$BRANCH_A" ]
    [ -d "/tmp/wt-$BRANCH_B" ]
    [ "/tmp/wt-$BRANCH_A" != "/tmp/wt-$BRANCH_B" ]
    # Confirm each worktree is on the expected branch.
    branch_a_head="$(git -C "/tmp/wt-$BRANCH_A" rev-parse --abbrev-ref HEAD)"
    branch_b_head="$(git -C "/tmp/wt-$BRANCH_B" rev-parse --abbrev-ref HEAD)"
    [ "$branch_a_head" = "$BRANCH_A" ]
    [ "$branch_b_head" = "$BRANCH_B" ]
}
