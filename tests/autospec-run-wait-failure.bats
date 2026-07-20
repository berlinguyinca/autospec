#!/usr/bin/env bats

setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"; }

@test "autospec-run outer Wait owner routes closed stdin through typed recovery" {
  for file in SKILL.md codex/prompt.md opencode/agent.md; do
    skill="$ROOT/skills/autospec-run/$file"
    wait_line="$(grep -nF '  wait for task-notification (monitor agent completes)' "$skill" | cut -d: -f1)"
    recovery_line="$(grep -nF '  if Wait returns `write_stdin failed` with `stdin is closed`:' "$skill" | cut -d: -f1)"
    consume_line="$(grep -nF '  # Read and consume the batch-done signal.' "$skill" | cut -d: -f1)"
    prompt_line="$(grep -nF '> **Prompt construction (cache-prefix + dynamic suffix):**' "$skill" | cut -d: -f1)"

    [ -n "$wait_line" ]
    [ -n "$recovery_line" ]
    [ -n "$consume_line" ]
    [ "$wait_line" -lt "$recovery_line" ]
    [ "$recovery_line" -lt "$consume_line" ]
    [ "$recovery_line" -lt "$prompt_line" ]
    [ "$(grep -Fc 'autonomous implementer-wait-failed --repo {repo}' "$skill")" -eq 1 ]
    [ "$(sed -n "${recovery_line},${consume_line}p" "$skill" | grep -Fc 'never mutate labels inline or overwrite a successor claim')" -eq 1 ]
  done
}
