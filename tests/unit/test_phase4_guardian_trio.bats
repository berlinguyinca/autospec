#!/usr/bin/env bats
# tests/unit/test_phase4_guardian_trio.bats — byte-identity assertions for the
# guardian dispatch block across all 6 Phase 4 trio files (3 per skill × 2 skills).
# Per spec §5.2 lock-step trio and §8 testing strategy.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

# extract_guardian_block <file>
# Emits lines between the outermost <!-- guardian-block:begin --> and
# <!-- guardian-block:end --> markers, tracking nesting so that the inner
# marker pair inside the GCMT heredoc does not prematurely close the block.
extract_guardian_block() {
    awk '
        /<!-- guardian-block:begin -->/ { depth++; if (depth == 1) { next } }
        /<!-- guardian-block:end -->/   { if (depth == 1) { depth=0; next } depth-- }
        depth >= 1 { print }
    ' "$1"
}

line_of() {
    grep -nF "$1" "$2" | head -n 1 | cut -d: -f1
}

# ── skills/autospec ────────────────────────────────────────────────────────────

@test "autospec SKILL.md guardian block has required markers" {
    block="$(extract_guardian_block "$REPO_ROOT/skills/autospec/SKILL.md")"
    [ -n "$block" ] || { echo "guardian block empty in autospec/SKILL.md"; return 1; }
    printf '%s\n' "$block" | grep -q 'AUTOSPEC_NO_GUARDIAN' \
        || { echo "missing AUTOSPEC_NO_GUARDIAN in autospec/SKILL.md guardian block"; return 1; }
    printf '%s\n' "$block" | grep -qE 'GUARDIAN_PASS|LGTM' \
        || { echo "missing GUARDIAN_PASS or LGTM verdict in autospec/SKILL.md guardian block"; return 1; }
}

@test "autospec codex prompt guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/codex/prompt.md") \
        || { echo "GUARDIAN_PASS: autospec codex/prompt.md guardian block diverges from SKILL.md"; return 1; }
}

@test "autospec opencode agent guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/opencode/agent.md") \
        || { echo "GUARDIAN_PASS: autospec opencode/agent.md guardian block diverges from SKILL.md"; return 1; }
}

# ── skills/autospec-run ────────────────────────────────────────────────────────

@test "autospec-run SKILL.md guardian block has required markers" {
    block="$(extract_guardian_block "$REPO_ROOT/skills/autospec-run/SKILL.md")"
    [ -n "$block" ] || { echo "guardian block empty in autospec-run/SKILL.md"; return 1; }
    printf '%s\n' "$block" | grep -q 'AUTOSPEC_NO_GUARDIAN' \
        || { echo "missing AUTOSPEC_NO_GUARDIAN in autospec-run/SKILL.md guardian block"; return 1; }
    printf '%s\n' "$block" | grep -qE 'GUARDIAN_PASS|LGTM' \
        || { echo "missing GUARDIAN_PASS or LGTM verdict in autospec-run/SKILL.md guardian block"; return 1; }
}

@test "autospec-run codex prompt guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec-run"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/codex/prompt.md") \
        || { echo "GUARDIAN_PASS: autospec-run codex/prompt.md guardian block diverges from SKILL.md"; return 1; }
}

@test "autospec-run opencode agent guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec-run"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/opencode/agent.md") \
        || { echo "GUARDIAN_PASS: autospec-run opencode/agent.md guardian block diverges from SKILL.md"; return 1; }
}

# ── cross-skill byte-identity ──────────────────────────────────────────────────

@test "autospec vs autospec-run guardian blocks byte-equal" {
    diff \
        <(extract_guardian_block "$REPO_ROOT/skills/autospec/SKILL.md") \
        <(extract_guardian_block "$REPO_ROOT/skills/autospec-run/SKILL.md") \
        || { echo "GUARDIAN_PASS: autospec and autospec-run SKILL.md guardian blocks diverge"; return 1; }
}

@test "PR_SIZE autospec-run gates push and final merge with exact acceptance evidence" {
    for file in \
        "$REPO_ROOT/skills/autospec-run/SKILL.md" \
        "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
        grep -qF '<!-- pr-size-pre-push:begin -->' "$file" \
            || { echo "missing PR_SIZE pre-push gate in $file"; return 1; }
        grep -qF '<!-- pr-size-final-merge:begin -->' "$file" \
            || { echo "missing PR_SIZE final-merge gate in $file"; return 1; }
        grep -qF '401 changed lines' "$file" \
            || { echo "missing 401-line rejection in $file"; return 1; }
        grep -qF '9 raw files' "$file" \
            || { echo "missing 9-file rejection in $file"; return 1; }
        grep -qF '4 logical units' "$file" \
            || { echo "missing 4-unit rejection in $file"; return 1; }
        grep -qF 'git diff --binary "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID"' "$file" \
            || { echo "PR_SIZE does not lint the exact base-to-head diff in $file"; return 1; }
        grep -qF "grep -qxF 'INFO:PR_SIZE: acceptance'" "$file" \
            || { echo "PR_SIZE acceptance is not exact-line matched in $file"; return 1; }

        pre_push="$(line_of '<!-- pr-size-pre-push:begin -->' "$file")"
        push="$(line_of '> 5. Push:' "$file")"
        final_merge="$(line_of '<!-- pr-size-final-merge:begin -->' "$file")"
        guarded_merge="$(line_of 'if bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-guarded-merge.sh"' "$file")"
        [ "$pre_push" -lt "$push" ] \
            || { echo "PR_SIZE pre-push gate follows push in $file"; return 1; }
        [ "$final_merge" -lt "$guarded_merge" ] \
            || { echo "PR_SIZE final gate follows guarded merge in $file"; return 1; }
    done
}
