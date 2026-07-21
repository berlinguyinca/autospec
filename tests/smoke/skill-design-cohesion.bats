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

    run grep -F "When the request describes a chat or assistant that answers questions about domain entities backed by real data" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "require the design spec to capture the current page and entity scope before opening a full-page assistant view. The assistant must preserve that scope across pop-out and full-page routes" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "Require a backend tool registry that defines named tools, input and output schemas, authorization rules, a bounded per-turn call cap" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "how sanitized tool evidence is injected into the final provider prompt" <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "Separate deterministic keyword-triggered tool plans from a bounded model-planned diagnostic step; the model-planned step may run only when deterministic routing finds no plan." <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "Require the spec's Testing section to cover nested-context extraction, deterministic intent-keyword coverage, scoped-entity diagnostics, and provider refusal and fallback behavior." <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]

    run grep -F "When a question maps to a registered tool, the final assistant answer must be grounded in tool evidence, not retrieval citations alone." <<<"$DIAGNOSTIC_ASSISTANT"
    [ "$status" -eq 0 ]
}
