#!/usr/bin/env bats
# tests/autonomous/test_cross_directory_guardrail.bats — regression coverage for
# issue #1544. The immutable-verifier diff-guard must fire on test/eval-harness
# files that live in NESTED directories (e.g. skills/*/tests/*), not only the
# top-level tests/ tree covered by test_immutable_verifier.bats.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
GUARDRAILS="$REPO_ROOT/scripts/autonomous-guardrails.sh"

setup() {
    TMP="$(mktemp -d -t cross_directory_guardrail.XXXXXX)"
}

teardown() {
    rm -rf "$TMP"
}

@test "implementer-lane PR editing a nested skill test is rejected by diff-guard" {
    changed="$TMP/changed.txt"
    printf 'skills/autospec-doc/tests/test_doc_config.bats\n' > "$changed"
    [ -f "$changed" ]

    run bash "$GUARDRAILS" diff-guard --lane implementer --changed-files "$changed"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q '^REASON:immutable_verifier_modified$'
    printf '%s\n' "$output" | grep -q '^PATH:skills/autospec-doc/tests/test_doc_config.bats$'
}
