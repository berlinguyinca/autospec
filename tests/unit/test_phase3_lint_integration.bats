#!/usr/bin/env bats
# tests/unit/test_phase3_lint_integration.bats — grep assertions verifying that
# every trio file in skills/autospec, skills/autospec-define, and
# skills/autospec-classify mentions the Phase 3 lint loop and Phase 3.5 audit
# keywords (installed lint-issue.sh helper path, needs-quality-bar, ## Quality lint).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

# ── skills/autospec ────────────────────────────────────────────────────────────

@test "autospec Phase 3 trio: SKILL.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec/SKILL.md"
}

@test "autospec Phase 3 trio: codex/prompt.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec/codex/prompt.md"
}

@test "autospec Phase 3 trio: opencode/agent.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec/opencode/agent.md"
}

@test "autospec Phase 3.5 trio: SKILL.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec/SKILL.md"
}

@test "autospec Phase 3.5 trio: SKILL.md mentions ## Quality lint" {
    grep -q '## Quality lint' "$REPO_ROOT/skills/autospec/SKILL.md"
}

@test "autospec Phase 3.5 trio: codex/prompt.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec/codex/prompt.md"
}

@test "autospec Phase 3.5 trio: opencode/agent.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec/opencode/agent.md"
}

# ── skills/autospec-define ────────────────────────────────────────────────────

@test "autospec-define Phase 3 trio: SKILL.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define Phase 3 trio: codex/prompt.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec-define/codex/prompt.md"
}

@test "autospec-define Phase 3 trio: opencode/agent.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
}

@test "autospec-define Phase 3.5 trio: SKILL.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define Phase 3.5 trio: SKILL.md mentions ## Quality lint" {
    grep -q '## Quality lint' "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define Phase 3.5 trio: codex/prompt.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-define/codex/prompt.md"
}

@test "autospec-define Phase 3.5 trio: opencode/agent.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-define/opencode/agent.md"
}

# ── skills/autospec-classify ──────────────────────────────────────────────────

@test "autospec-classify audit trio: SKILL.md mentions installed lint-issue.sh" {
    grep -q 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' "$REPO_ROOT/skills/autospec-classify/SKILL.md"
}

@test "autospec-classify audit trio: SKILL.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-classify/SKILL.md"
}

@test "autospec-classify audit trio: SKILL.md mentions ## Quality lint" {
    grep -q '## Quality lint' "$REPO_ROOT/skills/autospec-classify/SKILL.md"
}

@test "autospec-classify audit trio: codex/prompt.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-classify/codex/prompt.md"
}

@test "autospec-classify audit trio: opencode/agent.md mentions needs-quality-bar" {
    grep -q 'needs-quality-bar' "$REPO_ROOT/skills/autospec-classify/opencode/agent.md"
}

# ── cross-skill lock-step parity ──────────────────────────────────────────────

@test "lock-step: all 3 autospec-define harness files mention installed lint-issue.sh" {
    COUNT=$(grep -rl 'AUTOSPEC_SCRIPTS_DIR.*lint-issue.sh' \
        "$REPO_ROOT/skills/autospec-define/SKILL.md" \
        "$REPO_ROOT/skills/autospec-define/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-define/opencode/agent.md" | wc -l | tr -d ' ')
    [ "$COUNT" -eq 3 ]
}

@test "lock-step: all 3 autospec-classify harness files mention needs-quality-bar" {
    COUNT=$(grep -rl 'needs-quality-bar' \
        "$REPO_ROOT/skills/autospec-classify/SKILL.md" \
        "$REPO_ROOT/skills/autospec-classify/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-classify/opencode/agent.md" | wc -l | tr -d ' ')
    [ "$COUNT" -eq 3 ]
}

@test "autospec classify prompts require issue intent safety gate before auto-implement" {
    for file in \
        "$REPO_ROOT/skills/autospec-classify/SKILL.md" \
        "$REPO_ROOT/skills/autospec/SKILL.md" \
        "$REPO_ROOT/skills/autospec-define/SKILL.md"
    do
        grep -q "Issue intent safety gate" "$file"
        grep -q "scripts/lint-issue-safety.sh" "$file"
        grep -q "security:quarantined" "$file"
        grep -q "safety:reviewed" "$file"
        grep -q "remove-label auto-implement" "$file"
        grep -q "remove-label needs-classify" "$file"
    done
}

@test "autospec classify prompts place the safety gate before queue-preserving phase 3.5 steps" {
    assert_before() {
        local needle_a="$1"
        local needle_b="$2"
        local file="$3"
        local line_a
        local line_b

        line_a="$(grep -nF "$needle_a" "$file" | head -n1 | cut -d: -f1)"
        line_b="$(grep -nF "$needle_b" "$file" | head -n1 | cut -d: -f1)"

        [ "$line_a" -lt "$line_b" ]
    }

    assert_before '### Issue intent safety gate' '### Label transition for `needs-classify` issues' \
        "$REPO_ROOT/skills/autospec-classify/SKILL.md"
    assert_before '### Issue intent safety gate' 'Apply labels.' \
        "$REPO_ROOT/skills/autospec-classify/SKILL.md"
    assert_before '### Issue intent safety gate' 'Apply labels.' \
        "$REPO_ROOT/skills/autospec/SKILL.md"
    assert_before '### Issue intent safety gate' 'Apply labels.' \
        "$REPO_ROOT/skills/autospec-define/SKILL.md"
    assert_before '### Issue intent safety gate' '7. **Dependency-edge sanity checks.**' \
        "$REPO_ROOT/skills/autospec/SKILL.md"
    assert_before '### Issue intent safety gate' 'Board assignment' \
        "$REPO_ROOT/skills/autospec/SKILL.md"
    assert_before '### Issue intent safety gate' '7. **Dependency-edge sanity checks.**' \
        "$REPO_ROOT/skills/autospec-define/SKILL.md"
    assert_before '### Issue intent safety gate' 'Board assignment' \
        "$REPO_ROOT/skills/autospec-define/SKILL.md"
}
