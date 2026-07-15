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

@test "phase 3 prompts run safety pre-filing retry before gh issue create" {
    assert_before() {
        local needle_a="$1"
        local needle_b="$2"
        local file="$3"
        local line_a
        local line_b

        line_a="$(grep -nF "$needle_a" "$file" | head -n1 | cut -d: -f1)"
        line_b="$(grep -nF "$needle_b" "$file" | head -n1 | cut -d: -f1)"

        [ -n "$line_a" ] && [ -n "$line_b" ] && [ "$line_a" -lt "$line_b" ]
    }

    for file in "$REPO_ROOT/skills/autospec/SKILL.md" "$REPO_ROOT/skills/autospec-define/SKILL.md"; do
        assert_before "Pre-filing lint loop" "Pre-filing safety loop" "$file"
        grep -q 'On pass (exit 0), proceed to the safety loop' "$file"
        ! grep -q 'proceed to `gh issue create` as normal' "$file"
        grep -q "Pre-filing safety loop" "$file"
        grep -q "MAX_SAFETY_RETRIES=5" "$file"
        grep -q 'lint issue safety' "$file"
        grep -q "skip that child" "$file"
        grep -q 'after the issue-quality lint passes and before `gh issue create`' "$file"
    done
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

@test "phase 3.5 prompts delegate each automatic safety decision to exact Rust review" {
    for skill in autospec-classify autospec autospec-define; do
        "$REPO_ROOT/scripts/derive-trio.sh" "$REPO_ROOT/skills/$skill" --check
        for file in \
            "$REPO_ROOT/skills/$skill/SKILL.md" \
            "$REPO_ROOT/skills/$skill/codex/prompt.md" \
            "$REPO_ROOT/skills/$skill/opencode/agent.md"
        do
            grep -Fq 'queue review-safety --repo {repo} --limit 1 --issue <N>' "$file"
            ! grep -Fq 'safety:reviewed' "$file"
            ! grep -Fq 'security:quarantined' "$file"
            ! grep -Fq 'autospec-safety:begin' "$file"
            ! grep -Fq 'autospec-safety:end' "$file"
        done
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

@test "classification prompts keep pre-filing safety feedback separate from Rust writeback" {
    for file in "$REPO_ROOT/skills/autospec/SKILL.md" "$REPO_ROOT/skills/autospec-define/SKILL.md"; do
        grep -Fq 'Pre-filing safety loop' "$file"
        grep -Fq 'lint issue safety' "$file"
    done
}

@test "autospec-run identifies the Rust queue command as the only safety writer" {
    for file in \
        "$REPO_ROOT/skills/autospec-run/SKILL.md" \
        "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
    do
        grep -Fq 'queue review-safety' "$file"
        grep -Fq 'only automatic writer' "$file"
        ! grep -Fq 'Generate that decision through `autospec lint issue safety`' "$file"
    done
}

@test "autospec-explore prompts require exact Rust safety review before filing success" {
    "$REPO_ROOT/scripts/derive-trio.sh" "$REPO_ROOT/skills/autospec-explore" --check
    for file in \
        "$REPO_ROOT/skills/autospec-explore/SKILL.md" \
        "$REPO_ROOT/skills/autospec-explore/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-explore/opencode/agent.md"
    do
        grep -Fq 'queue review-safety --repo {repo} --limit 1 --issue <N>' "$file"
        ! grep -Fq 'safety:reviewed' "$file"
        ! grep -Fq 'security:quarantined' "$file"
        ! grep -Fq 'autospec-safety:begin' "$file"
        ! grep -Fq 'autospec-safety:end' "$file"
        ! grep -Fq 'autospec:needs-human' "$file"
        ! grep -Fq 'autospec-safety-decision:begin' "$file"
        ! grep -Fq 'autospec-safety-decision:end' "$file"
    done
}

@test "docs mention issue intent safety gate" {
    grep -q "lint issue safety" "$REPO_ROOT/docs/API_REFERENCE.md"
    grep -q "issue_intent_gate" "$REPO_ROOT/docs/CONFIG_REFERENCE.md"
    grep -q "security:quarantined" "$REPO_ROOT/docs/USER_MANUAL.md"
}
