#!/usr/bin/env bats
# tests/autospec-run/test_parallel_dispatch.bats — parallel implementer
# worktree isolation contract (issue #690 + #960).
#
# Fixtures (per spec):
#   a. helper creates worktree at /tmp/wt-<branch>
#   b. prompt pre-pend includes worktree path
#   c. helper removes worktree on completion (--cleanup)
#   d. two parallel invocations don't collide (distinct worktrees, distinct branches)
#   e. (new #960) ladder verdict 'fresh' surfaced when no --repo given
#   f. (new #960) ladder verdict 'branch-only' surfaced via mocked git ls-remote
#   g. (new #960) ladder verdict 'open-pr' surfaced via mocked gh pr list
#   h. (new #960) dirty-reuse refusal propagates (exit 4 + code_health marker)
#   i. (new #960) clean same-branch reuse still works (idempotent create)

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    HELPER="$REPO_ROOT/scripts/dispatch-implementer.sh"
    GUARD="$REPO_ROOT/scripts/worktree-guard.sh"
    TMPDIR_BATS="$(mktemp -d)"
    PROMPT_FILE="$TMPDIR_BATS/prompt.md"
    printf 'IMPLEMENTER_PROMPT_BODY\n' > "$PROMPT_FILE"
    BRANCH_A="test-690-fixture-a-$$"
    BRANCH_B="test-690-fixture-b-$$"
    MOCK_BIN="$TMPDIR_BATS/bin"
    mkdir -p "$MOCK_BIN"
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

@test "fixture e (#960): ladder verdict 'fresh' surfaced when no --repo given" {
    run bash "$HELPER" --issue 960 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE"
    [ "$status" -eq 0 ]
    [[ "$output" == *'branch_verdict='* ]]
    [[ "$output" == *'"state":"fresh"'* ]]
    [[ "$output" == *"Branch verdict:"* ]]
}

@test "fixture f (#960): ladder verdict 'branch-only' surfaced via mocked git ls-remote" {
    # Locate the real git binary BEFORE prepending the mock bin to PATH.
    REAL_GIT="$(command -v git)"
    REAL_GH="$(command -v gh)"

    # Mock git: ls-remote --heads returns a ref line (branch exists remotely);
    # fetch is stubbed to avoid a real network call; all other git commands
    # delegate to the real git binary (by absolute path, not via PATH lookup).
    cat > "$MOCK_BIN/git" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "ls-remote" ] && [ "\$2" = "--heads" ]; then
    printf 'abc123\trefs/heads/%s\n' "\${4:-branch}"
    exit 0
fi
if [ "\$1" = "fetch" ]; then
    exit 0
fi
exec "$REAL_GIT" "\$@"
MOCKEOF
    chmod +x "$MOCK_BIN/git"

    # Mock gh: no open PRs so the open-pr rung doesn't fire first.
    cat > "$MOCK_BIN/gh" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "pr" ] && [ "\$2" = "list" ]; then
    printf '[]\n'
    exit 0
fi
exec "$REAL_GH" "\$@"
MOCKEOF
    chmod +x "$MOCK_BIN/gh"

    run env PATH="$MOCK_BIN:$PATH" bash "$HELPER" \
        --issue 960 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE" \
        --repo "berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"branch-only"'* ]]
    [[ "$output" == *"Branch verdict:"* ]]
}

@test "fixture g (#960): ladder verdict 'open-pr' surfaced via mocked gh pr list" {
    # Locate the real binaries BEFORE prepending the mock bin to PATH.
    REAL_GIT="$(command -v git)"
    REAL_GH="$(command -v gh)"

    # Mock gh: return one open PR with number 42.
    cat > "$MOCK_BIN/gh" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "pr" ] && [ "\$2" = "list" ]; then
    printf '[{"number":42}]\n'
    exit 0
fi
exec "$REAL_GH" "\$@"
MOCKEOF
    chmod +x "$MOCK_BIN/gh"

    # Mock git: stub fetch to avoid a real network call (create calls git fetch origin).
    cat > "$MOCK_BIN/git" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "fetch" ]; then
    exit 0
fi
exec "$REAL_GIT" "\$@"
MOCKEOF
    chmod +x "$MOCK_BIN/git"

    run env PATH="$MOCK_BIN:$PATH" bash "$HELPER" \
        --issue 960 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE" \
        --repo "berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"open-pr"'* ]]
    [[ "$output" == *'"pr":42'* ]]
    [[ "$output" == *"Branch verdict:"* ]]
}

@test "fixture h (#960): dirty-reuse refusal propagates (exit 4 + code_health marker)" {
    DIRTY_BRANCH="test-960-dirty-$$"
    DIRTY_PATH="/tmp/wt-$DIRTY_BRANCH"

    # Create a real worktree, then dirty it with an untracked file.
    git worktree add -b "$DIRTY_BRANCH" "$DIRTY_PATH" origin/main >/dev/null 2>&1
    printf 'dirty-file\n' > "$DIRTY_PATH/dirty-untracked.txt"

    run bash "$HELPER" --issue 960 --branch "$DIRTY_BRANCH" --prompt-file "$PROMPT_FILE"

    # Cleanup regardless of test outcome.
    git worktree remove --force "$DIRTY_PATH" 2>/dev/null || rm -rf "$DIRTY_PATH"
    git worktree prune 2>/dev/null || true

    [ "$status" -eq 4 ]
    # The code_health marker must appear or the propagated error message.
    [[ "$output" == *"code_health:worktree_dirty_reuse_refused"* ]] \
        || [[ "$output" == *"worktree-guard.sh create exited 4"* ]]
}

@test "fixture i (#960): clean same-branch reuse still succeeds (idempotent create)" {
    # First call: creates the worktree.
    bash "$HELPER" --issue 960 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE" >/dev/null
    [ -d "/tmp/wt-$BRANCH_A" ]

    # Second call: same branch, clean worktree — must succeed (idempotent reuse).
    run bash "$HELPER" --issue 960 --branch "$BRANCH_A" --prompt-file "$PROMPT_FILE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/tmp/wt-$BRANCH_A"* ]]
    [[ "$output" == *"IMPLEMENTER_PROMPT_BODY"* ]]
}
