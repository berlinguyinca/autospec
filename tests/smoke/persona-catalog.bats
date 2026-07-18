#!/usr/bin/env bats
# Smoke wrapper for issue #1728's persona catalog loader.

if [ -z "${BATS_TEST_DIRNAME:-}" ]; then
    exec bats "$0" "$@"
fi

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "persona catalog focused unit suite passes" {
    run bash "$REPO_ROOT/tests/unit/persona-catalog.bats"
    [ "$status" -eq 0 ]
}
