#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "interactive autonomous invocation follows by default" {
  run grep -F 'autospec-autonomous start --follow --repo-dir "$PWD"' \
    "$REPO_ROOT/skills/autospec-autonomous/SKILL.md"
  [ "$status" -eq 0 ]
}

@test "explicit launch modes override the interactive follow default" {
  run grep -F '`--detach` or `--foreground`' \
    "$REPO_ROOT/skills/autospec-autonomous/SKILL.md"
  [ "$status" -eq 0 ]
}

@test "autonomous skill adapters remain derived from the canonical body" {
  run "$REPO_ROOT/scripts/derive-trio.sh" \
    "$REPO_ROOT/skills/autospec-autonomous" --check
  [ "$status" -eq 0 ]
}
