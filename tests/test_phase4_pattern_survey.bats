#!/usr/bin/env bats
# tests/test_phase4_pattern_survey.bats — regression for feat(autospec-run) #803
#
# The Phase 4 implementer prompt must contain a mandatory "Pattern survey"
# section so implementers survey analogous utilities before writing code,
# preventing duplicate helpers and enforcing explicit reuse decisions.

PHASE4_PROMPT="${BATS_TEST_DIRNAME}/../skills/autospec-run/prompts/phase4-implementer.md"

@test "phase4-implementer.md contains a Pattern survey section" {
    run grep -c "Pattern survey" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "Pattern survey section appears before Expand section" {
    survey_line=$(grep -n "Pattern survey" "$PHASE4_PROMPT" | head -1 | cut -d: -f1)
    expand_line=$(grep -n "^## Expand" "$PHASE4_PROMPT" | head -1 | cut -d: -f1)
    [ -n "$survey_line" ]
    [ -n "$expand_line" ]
    [ "$survey_line" -lt "$expand_line" ]
}

@test "Pattern survey section documents the reuse/no-reuse decision requirement" {
    run grep -c "Reusing\|No reuse" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}
