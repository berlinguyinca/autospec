#!/usr/bin/env bats
# tests/validate-remediation-section-coverage.bats — regression coverage for scripts/validate.sh
# remediation section guards (issue #1928).
#
# Implementation note: issue #1928 cites Adversarial verify evidence that
# check_gap_remediation_section and check_review_remediation_section had no Bats
# references. This single Bats guard covers both remediation-section validators.
# TDD red evidence: the first focused run failed because
# run_check_remediation_sections_in_tree was intentionally undefined.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/validate.sh"

extract_validate_func() {
    local func="$1"
    awk -v func="$func" '
        $0 ~ "^" func "\\(\\) \\{" {capture=1}
        capture {print}
        capture && /^}$/ {exit}
    ' "$SCRIPT"
}

extract_remediation_funcs() {
    extract_validate_func check_gap_remediation_section
    extract_validate_func check_review_remediation_section
}

run_check_remediation_sections_in_tree() {
    local tree="$1"
    (
        cd "$tree"
        bash -c "
            set -u
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            $(extract_remediation_funcs)
            check_gap_remediation_section
            check_review_remediation_section
        "
    )
}

new_remediation_tree() {
    local t trio
    t="$(mktemp -d)"
    mkdir -p "$t/skills/autospec-run/opencode" \
        "$t/skills/autospec-run/codex" \
        "$t/skills/autospec-review/opencode" \
        "$t/skills/autospec-review/codex"

    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        cat > "$t/skills/autospec-run/$trio" <<'EOF2'
# autospec-run fixture

## Phase 5.5 — End-of-run gap remediation

Gap remediation fixture content.
EOF2
        cat > "$t/skills/autospec-review/$trio" <<'EOF2'
# autospec-review fixture

## Remediation mode

Review remediation fixture content.
EOF2
    done
    printf '%s\n' "$t"
}

teardown() {
    [ -n "${TREE:-}" ] && rm -rf "$TREE"
    return 0
}

@test "remediation section validators pass when gap and review trios carry required sections" {
    TREE="$(new_remediation_tree)"

    run run_check_remediation_sections_in_tree "$TREE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"gap-remediation: autospec-run"* ]]
    [[ "$output" == *"review-remediation: autospec-review"* ]]
}

@test "remediation section validators fail when autospec-run lacks gap remediation" {
    TREE="$(new_remediation_tree)"
    sed -i.bak '/^## Phase 5[.]5 — End-of-run gap remediation$/d' \
        "$TREE/skills/autospec-run/codex/prompt.md"

    run run_check_remediation_sections_in_tree "$TREE"

    [ "$status" -ne 0 ]
    [[ "$output" == *"autospec-run: codex/prompt.md missing '## Phase 5.5 — End-of-run gap remediation' section"* ]]
}

@test "remediation section validators fail when autospec-review lacks remediation mode" {
    TREE="$(new_remediation_tree)"
    sed -i.bak '/^## Remediation mode$/d' \
        "$TREE/skills/autospec-review/opencode/agent.md"

    run run_check_remediation_sections_in_tree "$TREE"

    [ "$status" -ne 0 ]
    [[ "$output" == *"autospec-review: opencode/agent.md missing '## Remediation mode' section"* ]]
}
