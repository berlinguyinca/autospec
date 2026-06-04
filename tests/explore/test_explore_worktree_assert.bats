#!/usr/bin/env bats
# Issue #964 — autospec-explore trio worktree assert (D4 split).
#
# Asserts that every file in the autospec-explore trio
# (SKILL.md + codex/prompt.md + opencode/agent.md) carries the mandatory
# `worktree-guard.sh assert` call before any sandbox commit step, and that
# the Sandbox branch contract section is byte-identical across all three files.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL="$REPO_ROOT/skills/autospec-explore/SKILL.md"
    CODEX="$REPO_ROOT/skills/autospec-explore/codex/prompt.md"
    OPENCODE="$REPO_ROOT/skills/autospec-explore/opencode/agent.md"
}

# ---------------------------------------------------------------------------
# Named-content: worktree-guard.sh assert present in each trio file
# ---------------------------------------------------------------------------

@test "SKILL.md: worktree-guard.sh assert present in Sandbox branch contract" {
    grep -q 'worktree-guard.sh assert' "$SKILL"
}

@test "codex/prompt.md: worktree-guard.sh assert present in Sandbox branch contract" {
    grep -q 'worktree-guard.sh assert' "$CODEX"
}

@test "opencode/agent.md: worktree-guard.sh assert present in Sandbox branch contract" {
    grep -q 'worktree-guard.sh assert' "$OPENCODE"
}

# ---------------------------------------------------------------------------
# Named-content: MUST exit 0 guard requirement present in each trio file
# ---------------------------------------------------------------------------

@test "SKILL.md: MUST exit 0 gate wording present" {
    grep -q 'MUST exit 0' "$SKILL"
}

@test "codex/prompt.md: MUST exit 0 gate wording present" {
    grep -q 'MUST exit 0' "$CODEX"
}

@test "opencode/agent.md: MUST exit 0 gate wording present" {
    grep -q 'MUST exit 0' "$OPENCODE"
}

# ---------------------------------------------------------------------------
# Named-content: primary checkout hard rule present in each trio file
# ---------------------------------------------------------------------------

@test "SKILL.md: primary checkout read-only rule present" {
    grep -qi 'primary checkout' "$SKILL"
}

@test "codex/prompt.md: primary checkout read-only rule present" {
    grep -qi 'primary checkout' "$CODEX"
}

@test "opencode/agent.md: primary checkout read-only rule present" {
    grep -qi 'primary checkout' "$OPENCODE"
}

# ---------------------------------------------------------------------------
# Lock-step byte-identity: Sandbox branch contract section must be identical
# ---------------------------------------------------------------------------

@test "explore trio: Sandbox branch contract section is byte-identical across SKILL.md + codex/prompt.md + opencode/agent.md" {
    section_skill="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' "$SKILL")"
    section_codex="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' "$CODEX")"
    section_opencode="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' "$OPENCODE")"

    [ -n "$section_skill" ]   || { echo "SKILL.md: Sandbox branch contract section missing" >&2; return 1; }
    [ -n "$section_codex" ]   || { echo "codex/prompt.md: Sandbox branch contract section missing" >&2; return 1; }
    [ -n "$section_opencode" ] || { echo "opencode/agent.md: Sandbox branch contract section missing" >&2; return 1; }

    diff <(printf '%s\n' "$section_skill") <(printf '%s\n' "$section_codex") \
        || { echo "SKILL.md vs codex/prompt.md: Sandbox branch contract section differs" >&2; return 1; }

    diff <(printf '%s\n' "$section_skill") <(printf '%s\n' "$section_opencode") \
        || { echo "SKILL.md vs opencode/agent.md: Sandbox branch contract section differs" >&2; return 1; }
}
