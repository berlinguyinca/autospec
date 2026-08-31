#!/usr/bin/env bats
# tests/unit/test_agents_md_guardian_contract.bats — grep assertions verifying
# that AGENTS.md contains the ## Implementation-quality contract section, all
# 10 RULE_IDs, the directive map, the opt-out grammar regex, and the
# AUTOSPEC_NO_GUARDIAN=1 env var per spec §3 and §8.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AGENTS_MD="$REPO_ROOT/AGENTS.md"
    GUARDIAN_DESIGN="$REPO_ROOT/docs/specs/2026-05-02-autospec-implementation-guardian-design.md"
}

# ── Section heading ────────────────────────────────────────────────────────────

@test "AGENTS.md has Implementation-quality contract heading" {
    grep -q '^## Implementation-quality contract' "$AGENTS_MD"
}

# ── All 10 RULE_IDs ───────────────────────────────────────────────────────────

@test "AGENTS.md lists RULE_ID OUT_OF_SCOPE" {
    grep -q 'OUT_OF_SCOPE' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID MISSING_TEST" {
    grep -q 'MISSING_TEST' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID COMPLEXITY" {
    grep -q 'COMPLEXITY' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID SECURITY" {
    grep -q 'SECURITY' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID TODO_LEFT" {
    grep -q 'TODO_LEFT' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID MOCK_DB" {
    grep -q 'MOCK_DB' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID HALLUCINATED_API" {
    grep -q 'HALLUCINATED_API' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID DUPLICATE_CODE" {
    grep -q 'DUPLICATE_CODE' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID DOC_OUT_OF_SYNC" {
    grep -q 'DOC_OUT_OF_SYNC' "$AGENTS_MD"
}

@test "AGENTS.md lists RULE_ID INVENTED_CONFIG" {
    grep -q 'INVENTED_CONFIG' "$AGENTS_MD"
}

@test "AGENTS.md lists all 10 RULE_IDs" {
    RULE_IDS="OUT_OF_SCOPE MISSING_TEST COMPLEXITY SECURITY TODO_LEFT MOCK_DB HALLUCINATED_API DUPLICATE_CODE DOC_OUT_OF_SYNC INVENTED_CONFIG"
    for id in $RULE_IDS; do
        grep -q "$id" "$AGENTS_MD" || { echo "Missing RULE_ID: $id"; return 1; }
    done
}

# ── Corrective directive map ───────────────────────────────────────────────────

@test "AGENTS.md has corrective directive map heading" {
    grep -q 'Corrective directive map\|corrective directive' "$AGENTS_MD"
}

@test "OUT_OF_SCOPE guidance names the strict outline and Files-touched union" {
    for contract in "$AGENTS_MD" "$GUARDIAN_DESIGN"; do
        grep -q 'exact files or descendants of trailing-slash directories' "$contract"
        grep -q '## Files touched' "$contract"
        ! grep -q 'Revert other changes or amend the issue body' "$contract"
    done
}

@test "AGENTS.md directive map has Add a test phrase for MISSING_TEST" {
    grep -q 'Add a test under tests' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Split functions phrase for COMPLEXITY" {
    grep -q 'Split functions' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Remove the flagged pattern phrase for SECURITY" {
    grep -q 'Remove the flagged pattern' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Remove TODO phrase for TODO_LEFT" {
    grep -q 'Remove TODO/XXX/FIXME' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Remove DB mock phrase for MOCK_DB" {
    grep -q 'Remove DB mock' "$AGENTS_MD"
}

@test "AGENTS.md directive map has flagged symbol phrase for HALLUCINATED_API" {
    grep -q 'flagged symbol does not exist' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Reuse the existing helper phrase for DUPLICATE_CODE" {
    grep -q 'Reuse the existing helper' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Update the doc file phrase for DOC_OUT_OF_SYNC" {
    grep -q 'Update the doc file' "$AGENTS_MD"
}

@test "AGENTS.md directive map has Remove the invented phrase for INVENTED_CONFIG" {
    grep -q 'Remove the invented flag' "$AGENTS_MD"
}

# ── Opt-out grammar regex ─────────────────────────────────────────────────────

@test "AGENTS.md has opt-out grammar regex" {
    grep -qF '^Guardian:\s+(skip-' "$AGENTS_MD"
}

# ── Env var contract ──────────────────────────────────────────────────────────

@test "AGENTS.md has AUTOSPEC_NO_GUARDIAN=1 env var" {
    grep -q 'AUTOSPEC_NO_GUARDIAN=1' "$AGENTS_MD"
}

# ── Phase-mapping table ───────────────────────────────────────────────────────

@test "AGENTS.md phase-mapping table includes guardian Tier A row" {
    grep -q 'Implementation guardian.*A' "$AGENTS_MD"
}
