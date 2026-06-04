#!/usr/bin/env bats
# tests/release/test_release_worktree_assert.bats — release-trio worktree assert gate (D4, issue #965).
#
# Covers:
#  - The assert call is present before the commit step in all three release-trio files.
#  - The worktree-assert:begin/end sentinel block is present in all three files.
#  - The prose block is byte-identical across the trio (lock-step).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL="$REPO_ROOT/skills/autospec-release/SKILL.md"
    CODEX="$REPO_ROOT/skills/autospec-release/codex/prompt.md"
    OPENCODE="$REPO_ROOT/skills/autospec-release/opencode/agent.md"
}

@test "SKILL.md carries worktree-guard.sh assert" {
    grep -q 'worktree-guard.sh assert' "$SKILL"
}

@test "codex/prompt.md carries worktree-guard.sh assert" {
    grep -q 'worktree-guard.sh assert' "$CODEX"
}

@test "opencode/agent.md carries worktree-guard.sh assert" {
    grep -q 'worktree-guard.sh assert' "$OPENCODE"
}

@test "SKILL.md has MUST exit 0 wording" {
    grep -q 'MUST exit 0' "$SKILL"
}

@test "codex/prompt.md has MUST exit 0 wording" {
    grep -q 'MUST exit 0' "$CODEX"
}

@test "opencode/agent.md has MUST exit 0 wording" {
    grep -q 'MUST exit 0' "$OPENCODE"
}

@test "SKILL.md has worktree-assert sentinels" {
    grep -q 'worktree-assert:begin' "$SKILL"
    grep -q 'worktree-assert:end' "$SKILL"
}

@test "codex/prompt.md has worktree-assert sentinels" {
    grep -q 'worktree-assert:begin' "$CODEX"
    grep -q 'worktree-assert:end' "$CODEX"
}

@test "opencode/agent.md has worktree-assert sentinels" {
    grep -q 'worktree-assert:begin' "$OPENCODE"
    grep -q 'worktree-assert:end' "$OPENCODE"
}

@test "release-trio worktree-assert block is byte-identical across SKILL.md and codex/prompt.md" {
    skill_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' "$SKILL")"
    codex_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' "$CODEX")"
    [ "$skill_block" = "$codex_block" ]
}

@test "release-trio worktree-assert block is byte-identical across SKILL.md and opencode/agent.md" {
    skill_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' "$SKILL")"
    opencode_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' "$OPENCODE")"
    [ "$skill_block" = "$opencode_block" ]
}

@test "SKILL.md references primary checkout hard rule" {
    grep -qi 'primary checkout' "$SKILL"
}

@test "SKILL.md references git worktree remove cleanup" {
    grep -q 'git worktree remove' "$SKILL"
}

@test "SKILL.md references git worktree prune cleanup" {
    grep -q 'git worktree prune' "$SKILL"
}
