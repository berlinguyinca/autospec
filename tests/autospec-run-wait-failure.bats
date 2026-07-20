#!/usr/bin/env bats

setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"; }

@test "autospec-run trios route unrecoverable closed-stdin waits through typed recovery" {
  for file in SKILL.md codex/prompt.md opencode/agent.md; do
    run grep -F 'autonomous implementer-wait-failed --repo {repo}' "$ROOT/skills/autospec-run/$file"
    [ "$status" -eq 0 ]
    run grep -F 'actual session ID from the failed Wait target' "$ROOT/skills/autospec-run/$file"
    [ "$status" -eq 0 ]
  done
}
