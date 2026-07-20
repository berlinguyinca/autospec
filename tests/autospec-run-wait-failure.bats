#!/usr/bin/env bats

setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"; }

@test "autospec-run outer Wait owner routes closed stdin through typed recovery" {
  for file in SKILL.md codex/prompt.md opencode/agent.md; do
    skill="$ROOT/skills/autospec-run/$file"
    wait_line="$(grep -nF '  wait for task-notification (monitor agent completes)' "$skill" | cut -d: -f1)"
    recovery_line="$(grep -nF '  if Wait returns `write_stdin failed` with `stdin is closed`:' "$skill" | cut -d: -f1)"
    live_line="$(grep -nF '    if the child is reported live:' "$skill" | cut -d: -f1)"
    reap_line="$(grep -nF '      explicitly terminate and reap the child through the harness process API' "$skill" | cut -d: -f1)"
    fail_closed_line="$(grep -nF '      if termination and reap cannot be proven: stop without typed recovery or label mutation' "$skill" | cut -d: -f1)"
    typed_line="$(grep -nF '    run `"${AUTOSPEC_BIN:-autospec}" autonomous implementer-wait-failed' "$skill" | cut -d: -f1)"
    consume_line="$(grep -nF '  # Read and consume the batch-done signal.' "$skill" | cut -d: -f1)"
    prompt_line="$(grep -nF '> **Prompt construction (cache-prefix + dynamic suffix):**' "$skill" | cut -d: -f1)"

    [ -n "$wait_line" ]
    [ -n "$recovery_line" ]
    [ -n "$consume_line" ]
    [ "$wait_line" -lt "$recovery_line" ]
    [ "$recovery_line" -lt "$live_line" ]
    [ "$live_line" -lt "$reap_line" ]
    [ "$reap_line" -lt "$fail_closed_line" ]
    [ "$fail_closed_line" -lt "$typed_line" ]
    [ "$(sed -n "${recovery_line},${typed_line}p" "$skill" | grep -Fc 'read ISSUE, BRANCH, WORKER_ID, and CLAIM_ID from the active durable claim/heartbeat')" -eq 1 ]
    [ "$(sed -n "${typed_line}p" "$skill" | grep -Fc -- '--claim-id "<CLAIM_ID>"')" -eq 1 ]
    [ "$recovery_line" -lt "$consume_line" ]
    [ "$recovery_line" -lt "$prompt_line" ]
    [ "$(grep -Fc 'autonomous implementer-wait-failed --repo {repo}' "$skill")" -eq 1 ]
    [ "$(sed -n "${recovery_line},${consume_line}p" "$skill" | grep -Fc 'never mutate labels inline or overwrite a successor claim')" -eq 1 ]
  done
}
