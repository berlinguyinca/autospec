#!/usr/bin/env bats
# Regression coverage for issues #1992 and #2000: autospec-define must prompt
# website/app specs to define coherent app-wide design guidelines and prove
# positive design-system adoption before decomposition.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL="$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define Phase 2 includes website app Design cohesion guidance" {
    run grep -F "### Design cohesion" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "forms, filter bars, segmented" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "controls, tables, stat cards, empty states, charts, and navbars" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "full-document horizontal overflow" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "desktop and mobile visual QA" "$SKILL"
    [ "$status" -eq 0 ]
}

@test "autospec-define distinguishes cleanup from positive design-system adoption" {
    run grep -F "**Artifact cleanup.**" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "**Positive design-system adoption.**" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "canonical primitives and classes for page shells, filter panels, segmented" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "controls, date ranges, tables, empty states, and notices" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "an executable positive guard that fails when a raw toggle, date, or table" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "artifact absence and canonical primitive presence" "$SKILL"
    [ "$status" -eq 0 ]
}
