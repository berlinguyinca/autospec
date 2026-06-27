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

@test "phase4-implementer.md smoke gate scrubs gh admin creds from the subshell" {
    # The smoke command is extracted verbatim from the untrusted issue body and
    # runs moments before admin-merge under live gh creds (#W2). It must run in a
    # credential-scrubbed subshell so a poisoned smoke fence cannot reach
    # GH_TOKEN/GITHUB_TOKEN. Guard against a regression back to bare `eval`.
    run grep -c 'env -u GH_TOKEN -u GITHUB_TOKEN bash -c "\$smoke_cmd"' "$PHASE4_PROMPT"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
    # The unscrubbed `eval "$smoke_cmd"` form must no longer be present.
    run grep -c 'eval "\$smoke_cmd"' "$PHASE4_PROMPT"
    [ "$output" -eq 0 ]
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
