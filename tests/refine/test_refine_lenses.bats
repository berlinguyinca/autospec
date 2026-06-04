#!/usr/bin/env bats
# tests/refine/test_refine_lenses.bats — per-lens isolation tests (issue #670).
# Each lens applies a named, measurable text transformation.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"

setup() {
    # These tests assert the DETERMINISTIC template-lens output specifically.
    # Since the default lens mode is now auto/LLM-first (issue #1024), pin the
    # deterministic path so the template signatures are exercised regardless of
    # whether a claude/codex dispatcher is present in the test environment.
    export AUTOSPEC_REFINE_LENS_MODE=deterministic
    TEST_TMP="$(mktemp -d -t refine-lens.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    cat > "$REPO_ROOT/AGENTS.md" <<'EOF'
# AGENTS
Edit scripts/refine-prompt.sh and tests/refine/test_refine_lenses.bats.
Run scripts/validate.sh before commit.
EOF
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"
    ART_DIR="$TEST_TMP/artifacts"
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

read_refined() {
    local f
    f=$(ls "$ART_DIR"/*.json | head -1)
    jq -r '.rounds[0].refined_prompt' "$f"
}

@test "repo-grounding lens cites at least one file path from AGENTS.md" {
    run bash "$SCRIPT" "fix the thing" --rounds 1 --lenses repo-grounding --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local refined
    refined="$(read_refined)"
    [[ "$refined" == *"Repo grounding"* ]]
    [[ "$refined" == *"scripts/refine-prompt.sh"* || "$refined" == *"scripts/validate.sh"* ]]
}

@test "clarity-ac lens disambiguates hedges and adds AC checkboxes" {
    run bash "$SCRIPT" "we should probably maybe fix login" --rounds 1 --lenses clarity-ac --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local refined
    refined="$(read_refined)"
    [[ "$refined" == *"Acceptance criteria"* ]]
    [[ "$refined" == *"- [ ]"* ]]
    [[ "$refined" != *"should probably"* ]]
    [[ "$refined" != *"maybe"* ]]
    [[ "$refined" == *"MUST"* ]]
}

@test "sizing lens reports word count and enforces 400-word cap" {
    run bash "$SCRIPT" "short prompt" --rounds 1 --lenses sizing --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local refined
    refined="$(read_refined)"
    [[ "$refined" == *"Sizing"* ]]
    [[ "$refined" == *"Current word count:"* ]]
    [[ "$refined" == *"400-word cap"* || "$refined" == *"400 words"* || "$refined" == *"no split required"* ]]
}

@test "adversarial lens injects critical-question prompts" {
    run bash "$SCRIPT" "implement search" --rounds 1 --lenses adversarial --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local refined
    refined="$(read_refined)"
    [[ "$refined" == *"Adversarial review"* ]]
    [[ "$refined" == *"empty input"* ]]
    [[ "$refined" == *"malformed input"* ]]
    [[ "$refined" == *"forbidden paths"* ]]
}
