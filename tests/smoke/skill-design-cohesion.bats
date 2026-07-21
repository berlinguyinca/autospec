#!/usr/bin/env bats
# Regression coverage for issues #1992 and #2000: autospec-define must prompt
# website/app specs to define coherent app-wide design guidelines and prove
# positive design-system adoption before decomposition.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL="$REPO_ROOT/skills/autospec-define/SKILL.md"
    COHESION="$(sed -n '/^### Design cohesion$/,/^If this is a fresh repo/p' "$SKILL" | tr '\n' ' ')"
    DIAGNOSTIC_ASSISTANT="$(sed -n '/^### Diagnostic assistant$/,/^### /p' "$SKILL" | tr '\n' ' ')"
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

    run grep -F "remove inline styles, duplicate class attributes, and legacy table or card chrome" <<<"$COHESION"
    [ "$status" -eq 0 ]

    run grep -F "**Positive design-system adoption.**" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "canonical primitives and classes for page shells, filter panels, segmented" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "controls, date ranges, tables, empty states, and notices" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "use them instead of raw page-local layouts" <<<"$COHESION"
    [ "$status" -eq 0 ]

    run grep -F "an executable positive guard that fails when a raw toggle, date, or table" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "control does not use the project's design-system classes" <<<"$COHESION"
    [ "$status" -eq 0 ]

    run grep -F 'Cross-reference the `autospec-qa` revalidation plan' <<<"$COHESION"
    [ "$status" -eq 0 ]

    run grep -F "**UI and UX Behavior** item" <<<"$COHESION"
    [ "$status" -eq 0 ]

    run grep -F "artifact absence and canonical primitive presence" "$SKILL"
    [ "$status" -eq 0 ]
}

@test "autospec-define grounds entity-backed diagnostic assistants in bounded backend tools" {
    run grep -F "### Diagnostic assistant" "$SKILL"
    [ "$status" -eq 0 ]

    run grep -F "jobs, samples, logs, or incidents" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "page and entity scope" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "pop-out and full-page routes" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "named tools, input and output schemas, authorization rules" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "bounded per-turn call cap" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "sanitized tool evidence" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "deterministic keyword-triggered tool plans" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "only when deterministic routing finds no plan" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "nested-context extraction, deterministic intent-keyword coverage" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "scoped-entity diagnostics, and provider refusal and fallback behavior" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "grounded in tool evidence, not retrieval citations alone" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]
}
