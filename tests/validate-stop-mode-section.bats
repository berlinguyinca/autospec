#!/usr/bin/env bats
# tests/validate-stop-mode-section.bats — regression coverage for scripts/validate.sh
# check_stop_mode_section() (issue #1926).
#
# TDD red evidence: before adding the extraction/helper implementation below,
# this file failed because new_stop_mode_tree/run_check_stop_mode_section_in_tree
# did not exist; that proved the new Bats regression was active before coverage
# was completed.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/validate.sh"

extract_stop_mode_func() {
    awk '
        $0 ~ /^check_stop_mode_section\(\) \{/ {grab=1}
        grab {print}
        grab && /^\}$/ {grab=0}
    ' "$SCRIPT"
}

run_check_stop_mode_section_in_tree() {
    local tree="$1"
    (
        cd "$tree"
        bash -c "
            set -u
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            $(extract_stop_mode_func)
            check_stop_mode_section
        "
    )
}

new_stop_mode_tree() {
    local t skill trio
    t="$(mktemp -d)"
    for skill in autospec autospec-run autospec-stop; do
        mkdir -p "$t/skills/$skill/opencode" "$t/skills/$skill/codex"
        for trio in SKILL.md opencode/agent.md codex/prompt.md; do
            cat > "$t/skills/$skill/$trio" <<EOF2
---
name: $skill
---

## Stop mode

Dispatches stop requests through autospec-stop.sh.
EOF2
        done
    done
    printf '%s\n' "$t"
}

teardown() {
    [ -n "${TREE:-}" ] && rm -rf "$TREE"
    return 0
}

@test "check_stop_mode_section passes when every stop-mode trio has the section" {
    TREE="$(new_stop_mode_tree)"

    run run_check_stop_mode_section_in_tree "$TREE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"stop-mode: autospec"* ]]
    [[ "$output" == *"stop-mode: autospec-run"* ]]
    [[ "$output" == *"stop-mode: autospec-stop"* ]]
}

@test "check_stop_mode_section fails when autospec-run prompt lacks Stop mode" {
    TREE="$(new_stop_mode_tree)"
    sed -i.bak '/^## Stop mode$/d' "$TREE/skills/autospec-run/codex/prompt.md"

    run run_check_stop_mode_section_in_tree "$TREE"

    [ "$status" -ne 0 ]
    [[ "$output" == *"autospec-run: codex/prompt.md missing '## Stop mode' section"* ]]
}

@test "check_stop_mode_section skips absent target skills" {
    TREE="$(new_stop_mode_tree)"
    rm -rf "$TREE/skills/autospec-stop"

    run run_check_stop_mode_section_in_tree "$TREE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"stop-mode: autospec"* ]]
    [[ "$output" == *"stop-mode: autospec-run"* ]]
    [[ "$output" != *"stop-mode: autospec-stop"* ]]
}
