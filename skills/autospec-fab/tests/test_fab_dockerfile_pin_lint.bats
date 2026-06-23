#!/usr/bin/env bats
# test_fab_dockerfile_pin_lint.bats — host-portable pin-lint contract for the
# autospec-fab container Dockerfile (issue #1300, folded from deferred #1301).
# Runs WITHOUT building the image. Invoked by validate.sh check_autospec_fab_contract.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
LINT="$REPO_ROOT/scripts/lint-fab-dockerfile.sh"
DOCKERFILE="$REPO_ROOT/skills/autospec-fab/docker/Dockerfile"

setup() {
    TMP="$(mktemp -d)"
}
teardown() {
    rm -rf "$TMP"
}

@test "lint script exists and is bash -n clean" {
    [ -f "$LINT" ]
    run bash -n "$LINT"
    [ "$status" -eq 0 ]
}

@test "the shipped fab Dockerfile passes the pin lint" {
    [ -f "$DOCKERFILE" ]
    run bash "$LINT" "$DOCKERFILE"
    [ "$status" -eq 0 ]
}

@test "lint rejects a floating :latest tag" {
    printf 'FROM ubuntu:latest\nRUN apt-get install -y curl=1.0\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -ne 0 ]
}

@test "lint rejects an unpinned apt package" {
    printf 'FROM ubuntu@sha256:abc123\nRUN apt-get install -y curl\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -ne 0 ]
}

@test "lint rejects a non-digest FROM" {
    printf 'FROM ubuntu:24.04\nRUN apt-get install -y curl=1.0\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -ne 0 ]
}

@test "lint rejects pip install without --require-hashes" {
    printf 'FROM ubuntu@sha256:abc123\nRUN python3 -m pip install requests==2.0\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -ne 0 ]
}

@test "lint rejects an ARG digest default that is not a sha256" {
    printf 'ARG UBUNTU_DIGEST=24.04\nFROM ubuntu@${UBUNTU_DIGEST}\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -ne 0 ]
}

@test "lint accepts a clean ARG-digest multi-stage Dockerfile" {
    printf 'ARG UBUNTU_DIGEST=sha256:abc\nFROM ubuntu@${UBUNTU_DIGEST} AS base\nFROM base AS final\nRUN apt-get install -y curl=1.0\n' > "$TMP/D"
    run bash "$LINT" "$TMP/D"
    [ "$status" -eq 0 ]
}
