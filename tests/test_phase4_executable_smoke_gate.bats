#!/usr/bin/env bats
# tests/test_phase4_executable_smoke_gate.bats — regression for feat(autospec-run) #804
#
# The Phase 4 implementer prompt must contain a mandatory executable smoke
# test gate that runs before merge and fails the PR if smoke fails.

PHASE4_PROMPT="${BATS_TEST_DIRNAME}/../skills/autospec-run/prompts/phase4-implementer.md"
SPLIT_PROMPT="${BATS_TEST_DIRNAME}/../skills/autospec-split/codex/prompt.md"
DEFINE_PROMPT="${BATS_TEST_DIRNAME}/../skills/autospec-define/codex/prompt.md"

@test "phase4-implementer.md contains smoke_test_passes or eval smoke call" {
    run grep -c "smoke_test_passes\|eval.*smoke" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "phase4-implementer.md contains a Smoke test gate section" {
    run grep -c "Smoke test gate" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "phase4-implementer.md smoke gate aborts on failure with issue comment" {
    # Must comment on the issue AND exit 1 when smoke fails.
    run grep -c "exit 1" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "autospec-split decomposer enforces SMOKE_NOT_FENCED and SMOKE_MULTI_LINE" {
    run grep -c "SMOKE_NOT_FENCED\|SMOKE_MULTI_LINE" "$SPLIT_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "autospec-define decomposer enforces SMOKE_NOT_FENCED and SMOKE_MULTI_LINE" {
    run grep -c "SMOKE_NOT_FENCED\|SMOKE_MULTI_LINE" "$DEFINE_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

# Issue #3679: acceptance criteria must target the user's own route/command,
# never an adjacent endpoint; environmental limits need a discriminator.
DEFINE_SKILL="${BATS_TEST_DIRNAME}/../skills/autospec-define/SKILL.md"
DECOMPOSER_CONTRACT="${BATS_TEST_DIRNAME}/../skills/autospec-define/prompts/decomposer-contract.md"

@test "autospec-define spec template names the user's own route and bans adjacent-endpoint substitution" {
    for f in "$DEFINE_SKILL" "$DEFINE_PROMPT" "$DECOMPOSER_CONTRACT"; do
        run grep -c "the user's sentence is about" "$f"
        [ "$status" -eq 0 ]
        [ "$output" -ge 1 ]
        run grep -c "do not substitute an adjacent endpoint" "$f"
        [ "$status" -eq 0 ]
        [ "$output" -ge 1 ]
        run grep -c "choose a discriminator that isolates the property under test" "$f"
        [ "$status" -eq 0 ]
        [ "$output" -ge 1 ]
    done
}

@test "autospec-split spec template names the user's own route and bans adjacent-endpoint substitution" {
    run grep -c "the user's sentence is about" "$SPLIT_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
    run grep -c "do not substitute an adjacent endpoint" "$SPLIT_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
    run grep -c "choose a discriminator that isolates the property under test" "$SPLIT_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "phase4-implementer.md smoke gate verifies the user's own route with a discriminator" {
    # Patterns are line-anchored: the prompt paragraph is soft-wrapped.
    run grep -c "the user's requirement is about" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
    run grep -c "do NOT substitute an" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
    run grep -c "choose a discriminator that isolates the property under" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "phase4-implementer.md peer-review asks whether the evidence exercises the named requirement" {
    run grep -c "Does the evidence exercise the thing the requirement names, or something adjacent to it?" "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}
