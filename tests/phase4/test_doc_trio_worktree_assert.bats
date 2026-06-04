#!/usr/bin/env bats
# tests/phase4/test_doc_trio_worktree_assert.bats
# Verifies the doc-trio worktree-guard.sh assert wiring (issue #963, spec §D4):
#   1. All three doc-trio files carry `worktree-guard.sh assert` before the
#      regenerate commit step.
#   2. All three files carry the MUST-exit-0 rule.
#   3. All three files carry the doc-regen-assert sentinel block.
#   4. The new section is byte-identical across all three trio files (lock-step).

REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
SKILL="$REPO_ROOT/skills/autospec-doc"
SKILL_MD="$SKILL/SKILL.md"
CODEX_MD="$SKILL/codex/prompt.md"
OPENCODE_MD="$SKILL/opencode/agent.md"

# ── 1. worktree-guard.sh assert present in all three trio files ───────────────

@test "SKILL.md: worktree-guard.sh assert present before regenerate commit" {
    grep -q 'worktree-guard.sh assert' "$SKILL_MD"
}

@test "codex/prompt.md: worktree-guard.sh assert present before regenerate commit" {
    grep -q 'worktree-guard.sh assert' "$CODEX_MD"
}

@test "opencode/agent.md: worktree-guard.sh assert present before regenerate commit" {
    grep -q 'worktree-guard.sh assert' "$OPENCODE_MD"
}

# ── 2. MUST exit 0 rule present in all three trio files ──────────────────────

@test "SKILL.md: doc-regen assert gate requires MUST exit 0" {
    grep -q 'MUST exit 0' "$SKILL_MD"
}

@test "codex/prompt.md: doc-regen assert gate requires MUST exit 0" {
    grep -q 'MUST exit 0' "$CODEX_MD"
}

@test "opencode/agent.md: doc-regen assert gate requires MUST exit 0" {
    grep -q 'MUST exit 0' "$OPENCODE_MD"
}

# ── 3. doc-regen-assert sentinel block present in all three trio files ────────

@test "SKILL.md: doc-regen-assert:begin sentinel present" {
    grep -q 'doc-regen-assert:begin' "$SKILL_MD"
}

@test "codex/prompt.md: doc-regen-assert:begin sentinel present" {
    grep -q 'doc-regen-assert:begin' "$CODEX_MD"
}

@test "opencode/agent.md: doc-regen-assert:begin sentinel present" {
    grep -q 'doc-regen-assert:begin' "$OPENCODE_MD"
}

# ── 4. Lock-step byte-identity of the Worktree assert section ─────────────────

extract_assert_section() {
    awk '/^## Worktree assert \(regenerate commit\)$/{p=1} /^## Invocation$/{p=0} p{print}' "$1"
}

@test "doc-trio lock-step: SKILL.md and codex/prompt.md assert section byte-identical" {
    s1="$(extract_assert_section "$SKILL_MD")"
    s2="$(extract_assert_section "$CODEX_MD")"
    [ -n "$s1" ]
    [ "$s1" = "$s2" ]
}

@test "doc-trio lock-step: codex/prompt.md and opencode/agent.md assert section byte-identical" {
    s2="$(extract_assert_section "$CODEX_MD")"
    s3="$(extract_assert_section "$OPENCODE_MD")"
    [ -n "$s2" ]
    [ "$s2" = "$s3" ]
}
