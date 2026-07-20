#!/usr/bin/env bats
# Regression coverage for issue #1992: autospec-define must prompt website/app
# specs to define coherent app-wide design guidelines before decomposition.

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
