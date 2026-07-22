#!/usr/bin/env bats

@test "user manual generator avoids ambiguous any type token" {
    repo_root="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    run grep -nE '\\bany\\b' \
        "$repo_root/skills/autospec-shared/scripts/gen-docs/user-manual.mjs"
    [ "$status" -eq 1 ]
}
