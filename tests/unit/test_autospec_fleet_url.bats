#!/usr/bin/env bats
# tests/unit/test_autospec_fleet_url.bats - autospec-fleet URL normalization.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FLEET_LIB="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-lib.sh"
    FLEET_INIT="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-init.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-url-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

@test "normalize_repo_url accepts HTTPS without .git suffix" {
    run bash -c 'source "$1"; normalize_repo_url "https://github.com/org/repo"' _ "$FLEET_LIB"

    [ "$status" -eq 0 ]
    [ "$output" = "org/repo" ]
}

@test "normalize_repo_url accepts HTTPS with .git suffix" {
    run bash -c 'source "$1"; normalize_repo_url "https://github.com/org/repo.git"' _ "$FLEET_LIB"

    [ "$status" -eq 0 ]
    [ "$output" = "org/repo" ]
}

@test "normalize_repo_url accepts SSH with .git suffix" {
    run bash -c 'source "$1"; normalize_repo_url "git@github.com:org/repo.git"' _ "$FLEET_LIB"

    [ "$status" -eq 0 ]
    [ "$output" = "org/repo" ]
}

@test "repo_slug maps owner/repo to owner__repo" {
    run bash -c 'source "$1"; repo_slug "org/repo"' _ "$FLEET_LIB"

    [ "$status" -eq 0 ]
    [ "$output" = "org__repo" ]
}

@test "fleet-init dry-run prints deterministic checkout paths without cloning" {
    workspace="$TEST_TMPDIR/repos"

    run bash "$FLEET_INIT" --dry-run --workspace "$workspace" \
        "https://github.com/org/repo-a.git" \
        "git@github.com:org/repo-b.git"

    [ "$status" -eq 0 ]
    [[ "$output" == *"fleet: plan clone org/repo-a -> $workspace/org__repo-a"* ]]
    [[ "$output" == *"fleet: plan clone org/repo-b -> $workspace/org__repo-b"* ]]
    [ ! -e "$workspace" ]
}
