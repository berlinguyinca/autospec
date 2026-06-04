#!/usr/bin/env bats
# tests/qa/test_qa_documentation_gate.bats — QA documentation gate (issue #956, D3)
# Locks in:
#   1. '## Documentation gate' section is present in all three qa trio files
#   2. doc-orchestrator.mjs --full is referenced in the gate section
#   3. verify-examples.mjs is referenced in the gate section
#   4. check-doc-drift.sh --working-tree is referenced in the gate section
#   5. auto_regenerate is explicitly documented as NOT consulted
#   6. Graceful skip on orchestrator exit 2 is documented (INFO, not a finding)
#   7. Lock-step: frontmatter-stripped content is byte-identical across the trio

setup() {
    SKILL_DIR="$BATS_TEST_DIRNAME/../../skills/autospec-qa"
    SKILL_MD="$SKILL_DIR/SKILL.md"
    CODEX_MD="$SKILL_DIR/codex/prompt.md"
    OC_MD="$SKILL_DIR/opencode/agent.md"
}

# Helper: extract content after frontmatter (--- ... --- block).
strip_frontmatter() {
    local file="$1"
    python3 - "$file" <<'EOF'
import sys
lines = open(sys.argv[1]).readlines()
i = 0
if lines and lines[0].strip() == '---':
    i = 1
    while i < len(lines) and lines[i].strip() != '---':
        i += 1
    i += 1  # skip closing ---
sys.stdout.writelines(lines[i:])
EOF
}

@test "SKILL.md contains '## Documentation gate' section" {
    grep -q '^## Documentation gate' "$SKILL_MD"
}

@test "codex/prompt.md contains '## Documentation gate' section" {
    grep -q '^## Documentation gate' "$CODEX_MD"
}

@test "opencode/agent.md contains '## Documentation gate' section" {
    grep -q '^## Documentation gate' "$OC_MD"
}

@test "SKILL.md references doc-orchestrator.mjs --full in Documentation gate" {
    grep -q 'doc-orchestrator.mjs --full' "$SKILL_MD"
}

@test "SKILL.md references verify-examples.mjs in Documentation gate" {
    grep -q 'verify-examples.mjs' "$SKILL_MD"
}

@test "SKILL.md references check-doc-drift.sh --working-tree in Documentation gate" {
    grep -q 'check-doc-drift.sh --working-tree' "$SKILL_MD"
}

@test "SKILL.md states auto_regenerate is NOT consulted" {
    grep -qE 'auto_regenerate.*NOT consulted|NOT consulted.*auto_regenerate|auto_regenerate` is NOT consulted' "$SKILL_MD"
}

@test "SKILL.md documents graceful skip on orchestrator exit 2 (INFO, not a finding)" {
    grep -q 'exit 2\|exits 2\|orchestrator exits 2' "$SKILL_MD"
}

@test "SKILL.md documents that graceful skip is logged as INFO not a finding" {
    grep -q 'INFO' "$SKILL_MD"
}

@test "SKILL.md: Documentation gate failure classes include generation error" {
    grep -q 'Generation error\|generation error' "$SKILL_MD"
}

@test "SKILL.md: Documentation gate failure classes include failing example" {
    grep -q 'Failing example\|failing example' "$SKILL_MD"
}

@test "SKILL.md: Documentation gate failure classes include residual drift" {
    grep -q 'Residual drift\|residual drift' "$SKILL_MD"
}

@test "SKILL.md: Documentation gate failure classes include missing audience page" {
    grep -q 'Missing audience page\|missing audience page' "$SKILL_MD"
}

@test "trio lock-step: SKILL.md and codex/prompt.md have identical content after frontmatter strip" {
    command -v python3 >/dev/null 2>&1 || skip "python3 required for frontmatter stripping"
    local skill_content codex_content
    skill_content="$(strip_frontmatter "$SKILL_MD")"
    codex_content="$(strip_frontmatter "$CODEX_MD")"
    [ "$skill_content" = "$codex_content" ]
}

@test "trio lock-step: SKILL.md and opencode/agent.md have identical content after frontmatter strip" {
    command -v python3 >/dev/null 2>&1 || skip "python3 required for frontmatter stripping"
    local skill_content oc_content
    skill_content="$(strip_frontmatter "$SKILL_MD")"
    oc_content="$(strip_frontmatter "$OC_MD")"
    [ "$skill_content" = "$oc_content" ]
}
