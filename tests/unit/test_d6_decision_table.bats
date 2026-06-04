#!/usr/bin/env bats
# tests/unit/test_d6_decision_table.bats — verify the decision table doc
#
# Spec ref: docs/specs/2026-06-04-skill-token-efficiency-design.md §5 D6
# Issue: #1025

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TABLE="$REPO_ROOT/docs/reports/llm-touchpoint-decision-table.md"
}

# Positive: decision table file exists
@test "decision table file exists" {
    test -f "$TABLE"
}

# Positive: table has >= 8 pipe-separated rows (header + 8 data rows)
@test "table has at least 8 pipe characters (AC: grep -c '|' >= 8)" {
    count=$(grep -c '|' "$TABLE")
    [ "$count" -ge 8 ]
}

# Positive: all 8 touchpoints are present
@test "Phase 1 research row present" {
    grep -q "Phase 1 research" "$TABLE"
}

@test "Phase 2 brainstorm/design row present" {
    grep -q "Phase 2" "$TABLE"
}

@test "Phase 3 decompose row present" {
    grep -q "Phase 3 decompose" "$TABLE"
}

@test "Phase 3.5 classify row present" {
    grep -q "Phase 3.5" "$TABLE"
}

@test "Phase 4 implementer row present" {
    grep -q "Phase 4 implementer" "$TABLE"
}

@test "Phase 4 guardian+LGTM row present" {
    grep -q "guardian" "$TABLE"
}

@test "Phase 5.5 gap audit row present" {
    grep -q "Phase 5.5" "$TABLE"
}

@test "refine lenses row present with INVERT verdict" {
    grep -q "INVERT" "$TABLE"
}

# Positive: KEEP-LLM rows cite refine-lenses fixture path
@test "KEEP-LLM rows cite refine-lenses fixture path" {
    grep -q "tests/fixtures/quality-diff/refine-lenses" "$TABLE"
}

# Positive: refine-lenses row cites all three fixture subdirs
@test "refine-lenses row cites caching-profile-api fixture" {
    grep -q "caching-profile-api" "$TABLE"
}

@test "refine-lenses row cites csv-export-stream fixture" {
    grep -q "csv-export-stream" "$TABLE"
}

@test "refine-lenses row cites oauth-token-refresh fixture" {
    grep -q "oauth-token-refresh" "$TABLE"
}

# Positive: Phase 3.5 row cites classify-model-fit.sh
@test "Phase 3.5 row cites classify-model-fit.sh" {
    grep -q "classify-model-fit.sh" "$TABLE"
}

# Positive: doc has the default rule prose
@test "doc contains default KEEP-LLM-WITH-ESCALATION rule" {
    grep -q "KEEP-LLM-WITH-ESCALATION" "$TABLE"
}

# Positive: doc has re-run instructions
@test "doc contains quality-differential re-run command" {
    grep -q "quality-differential.sh" "$TABLE"
}

# Negative: a KEEP-LLM row missing a fixture path fails
@test "negative: a row with KEEP-LLM and no fixture-path column fails a grep" {
    # Construct a broken table line and confirm it lacks a fixture path ref
    bad_line="| Phase X | LLM | KEEP-LLM | Some rationale | |"
    # The real table must NOT have any data row with an empty last column
    # (i.e., no row ending in "| |")
    if echo "$bad_line" | grep -q "| |$"; then
        # If any real row in the table has this pattern, fail
        if grep -qP '\| \|$' "$TABLE" 2>/dev/null || grep -q '| |$' "$TABLE"; then
            false
        fi
    fi
    true
}
