#!/usr/bin/env bats
# tests/guardian/test_rule_ids.bats
#
# Asserts that the autospec-run reviewer prompt body in
# skills/autospec-run/SKILL.md names both new LLM-tier RULE_IDs introduced
# in issue #637:
#   - STRING_MATCH_DOMAIN_LOGIC
#   - REPEATED_STRUCTURE_AS_CODE
#
# This is the PR-time enforcement signal — the fused guardian/LGTM reviewer
# must enumerate both IDs in its LLM-tier scan list so the implementer retry
# loop has a chance to rewrite offending diffs before merge.

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."

@test "autospec-run SKILL.md reviewer prompt names STRING_MATCH_DOMAIN_LOGIC" {
    run grep -F 'STRING_MATCH_DOMAIN_LOGIC' "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "autospec-run SKILL.md reviewer prompt names REPEATED_STRUCTURE_AS_CODE" {
    run grep -F 'REPEATED_STRUCTURE_AS_CODE' "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "autospec-run codex/prompt.md names both new RULE_IDs" {
    run grep -F 'STRING_MATCH_DOMAIN_LOGIC' "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    [ "$status" -eq 0 ]
    run grep -F 'REPEATED_STRUCTURE_AS_CODE' "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    [ "$status" -eq 0 ]
}

@test "autospec-run opencode/agent.md names both new RULE_IDs" {
    run grep -F 'STRING_MATCH_DOMAIN_LOGIC' "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
    [ "$status" -eq 0 ]
    run grep -F 'REPEATED_STRUCTURE_AS_CODE' "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
    [ "$status" -eq 0 ]
}

@test "AGENTS.md RULE_ID table includes both new IDs" {
    run grep -F 'STRING_MATCH_DOMAIN_LOGIC' "$REPO_ROOT/AGENTS.md"
    [ "$status" -eq 0 ]
    run grep -F 'REPEATED_STRUCTURE_AS_CODE' "$REPO_ROOT/AGENTS.md"
    [ "$status" -eq 0 ]
}
