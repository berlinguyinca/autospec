#!/usr/bin/env bats
# tests/validate-enforcement-defaults-section.bats — regression coverage for
# scripts/validate.sh check_enforcement_defaults_section() (issue #1929).
#
# Adversarial verify evidence cited before editing: validate.sh defines
# check_enforcement_defaults_section but no Bats test referenced it.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/validate.sh"

extract_enforcement_defaults_func() {
    awk '
        $0 ~ /^check_enforcement_defaults_section\(\) \{/ {grab=1}
        grab {print}
        grab && /^\}$/ {grab=0}
    ' "$SCRIPT"
}

run_check_enforcement_defaults_section_in_tree() {
    local tree="$1"
    (
        cd "$tree"
        bash -c "
            set -u
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            $(extract_enforcement_defaults_func)
            check_enforcement_defaults_section
        "
    )
}

new_enforcement_defaults_tree() {
    local t trio
    t="$(mktemp -d)"
    mkdir -p "$t/skills/autospec-secaudit/opencode" "$t/skills/autospec-secaudit/codex"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        cat > "$t/skills/autospec-secaudit/$trio" <<'EOF2'
---
name: autospec-secaudit
---

## Enforcement defaults

Documents which security audit dimensions block versus remain advisory.
EOF2
    done
    printf '%s\n' "$t"
}

teardown() {
    [ -n "${TREE:-}" ] && rm -rf "$TREE"
    return 0
}

@test "check_enforcement_defaults_section passes when the secaudit trio has the section" {
    TREE="$(new_enforcement_defaults_tree)"

    run run_check_enforcement_defaults_section_in_tree "$TREE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"enforcement-defaults: autospec-secaudit"* ]]
}

@test "check_enforcement_defaults_section fails when codex prompt lacks Enforcement defaults" {
    TREE="$(new_enforcement_defaults_tree)"
    sed -i.bak '/^## Enforcement defaults$/d' "$TREE/skills/autospec-secaudit/codex/prompt.md"

    run run_check_enforcement_defaults_section_in_tree "$TREE"

    [ "$status" -ne 0 ]
    [[ "$output" == *"autospec-secaudit: codex/prompt.md missing '## Enforcement defaults' section"* ]]
}

@test "check_enforcement_defaults_section skips an absent autospec-secaudit skill" {
    TREE="$(new_enforcement_defaults_tree)"
    rm -rf "$TREE/skills/autospec-secaudit"

    run run_check_enforcement_defaults_section_in_tree "$TREE"

    [ "$status" -eq 0 ]
    [[ "$output" != *"enforcement-defaults: autospec-secaudit"* ]]
}
