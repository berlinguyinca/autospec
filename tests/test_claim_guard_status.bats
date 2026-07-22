#!/usr/bin/env bats
# Regression coverage for the claim status audit surface.

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

@test "claim status script avoids ambiguous audit token" {
    ! grep -Eq '\bany\b' "$GUARD"
}

@test "claim status reports an empty repository store" {
    state_dir="$(mktemp -d)"
    trap 'rm -rf "$state_dir"' EXIT
    export AUTOSPEC_STATE_DIR="$state_dir"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    run bash "$GUARD" status
    [ "$status" -eq 0 ]
    [[ "$output" == *"no live claims for this repo"* ]]
}
