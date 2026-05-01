#!/usr/bin/env bats
# tests/unit/test_spec_pr_auto_merge.bats — assert the §3.6 spec-PR auto-merge
# contract is present in both the autospec and autospec-define trios.
#
# Checks:
#   1. Happy path: autospec trio each contains the gh pr merge --admin step.
#   2. Happy path: autospec-define trio each contains the same step.
#   3. Opt-out: both trios reference AUTOSPEC_NO_AUTOMERGE_SPEC=1.
#
# Spec ref: docs/specs/2026-05-01-autospec-startup-self-update-design.md §3.6

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

setup() {
    export REPO_ROOT
}

# ---------------------------------------------------------------------------
# Happy path — autospec trio
# ---------------------------------------------------------------------------

@test "autospec/SKILL.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec/SKILL.md"
}

@test "autospec/opencode/agent.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec/opencode/agent.md"
}

@test "autospec/codex/prompt.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec/codex/prompt.md"
}

# ---------------------------------------------------------------------------
# Happy path — autospec-define trio
# ---------------------------------------------------------------------------

@test "autospec-define/SKILL.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define/opencode/agent.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
}

@test "autospec-define/codex/prompt.md: contains gh pr merge --admin --squash --delete-branch" {
    grep -q 'gh pr merge.*--admin.*--squash.*--delete-branch' \
        "$REPO_ROOT/skills/autospec-define/codex/prompt.md"
}

# ---------------------------------------------------------------------------
# Opt-out env var documented in both trios
# ---------------------------------------------------------------------------

@test "autospec trio: all files reference AUTOSPEC_NO_AUTOMERGE_SPEC" {
    for f in \
        "$REPO_ROOT/skills/autospec/SKILL.md" \
        "$REPO_ROOT/skills/autospec/opencode/agent.md" \
        "$REPO_ROOT/skills/autospec/codex/prompt.md"
    do
        grep -q 'AUTOSPEC_NO_AUTOMERGE_SPEC' "$f" \
            || { echo "AUTOSPEC_NO_AUTOMERGE_SPEC missing in $f" >&2; return 1; }
    done
}

@test "autospec-define trio: all files reference AUTOSPEC_NO_AUTOMERGE_SPEC" {
    for f in \
        "$REPO_ROOT/skills/autospec-define/SKILL.md" \
        "$REPO_ROOT/skills/autospec-define/opencode/agent.md" \
        "$REPO_ROOT/skills/autospec-define/codex/prompt.md"
    do
        grep -q 'AUTOSPEC_NO_AUTOMERGE_SPEC' "$f" \
            || { echo "AUTOSPEC_NO_AUTOMERGE_SPEC missing in $f" >&2; return 1; }
    done
}

# ---------------------------------------------------------------------------
# Step position: auto-merge step appears before Phase 3 in both SKILL.md files
# ---------------------------------------------------------------------------

@test "autospec/SKILL.md: spec-PR merge step precedes Phase 3 heading" {
    merge_line="$(grep -n 'AUTOSPEC_NO_AUTOMERGE_SPEC' "$REPO_ROOT/skills/autospec/SKILL.md" | head -1 | cut -d: -f1)"
    phase3_line="$(grep -n '^## Phase 3' "$REPO_ROOT/skills/autospec/SKILL.md" | head -1 | cut -d: -f1)"
    [ -n "$merge_line" ] && [ -n "$phase3_line" ]
    [ "$merge_line" -lt "$phase3_line" ]
}

@test "autospec-define/SKILL.md: spec-PR merge step precedes Phase 3 heading" {
    merge_line="$(grep -n 'AUTOSPEC_NO_AUTOMERGE_SPEC' "$REPO_ROOT/skills/autospec-define/SKILL.md" | head -1 | cut -d: -f1)"
    phase3_line="$(grep -n '^## Phase 3' "$REPO_ROOT/skills/autospec-define/SKILL.md" | head -1 | cut -d: -f1)"
    [ -n "$merge_line" ] && [ -n "$phase3_line" ]
    [ "$merge_line" -lt "$phase3_line" ]
}
