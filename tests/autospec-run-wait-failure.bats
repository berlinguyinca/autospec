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
    [ "$(sed -n "${recovery_line},${typed_line}p" "$skill" | grep -Fc 'read the immutable heartbeat binding for ACTUAL_SESSION_ID')" -eq 1 ]
    [ "$(sed -n "${recovery_line},${typed_line}p" "$skill" | grep -Fc 'heartbeat-read.sh" --repo {repo} --session-id "<ACTUAL_SESSION_ID>"')" -eq 1 ]
    [ "$(sed -n "${recovery_line},${typed_line}p" "$skill" | grep -Fc 'never read CLAIM_ID from the currently active claim after Wait fails')" -eq 1 ]
    [ "$(sed -n "${typed_line}p" "$skill" | grep -Fc -- '--claim-id "<CLAIM_ID>"')" -eq 1 ]
    [ "$recovery_line" -lt "$consume_line" ]
    [ "$recovery_line" -lt "$prompt_line" ]
    [ "$(grep -Fc 'autonomous implementer-wait-failed --repo {repo}' "$skill")" -eq 1 ]
    [ "$(sed -n "${recovery_line},${consume_line}p" "$skill" | grep -Fc 'never mutate labels inline or overwrite a successor claim')" -eq 1 ]
  done
}

@test "failed Wait recovery uses the old session generation after a same-owner successor" {
  hb_write="$ROOT/skills/autospec-run/scripts/heartbeat-write.sh"
  hb_read="$ROOT/skills/autospec-run/scripts/heartbeat-read.sh"
  autospec="$ROOT/target/debug/autospec"
  [ -x "$autospec" ] || cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p autospec-cli --bin autospec
  tmp="$(mktemp -d)"
  export AUTOSPEC_HEARTBEAT_DIR="$tmp/heartbeats"
  mkdir -p "$tmp/bin"

  bash "$hb_write" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
    --worker-id worker-a --claim-id claim-generation-old --session-id session-old
  bash "$hb_write" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
    --worker-id worker-a --claim-id claim-generation-new --session-id session-new
  binding="$(bash "$hb_read" --repo testorg/testrepo --session-id session-old)"
  bound_claim_id="$(printf '%s' "$binding" | jq -r .claim_id)"
  [ "$bound_claim_id" = claim-generation-old ]

  cat > "$tmp/bin/gh" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_WAIT_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat <<'JSON'
[{"id":100,"updated_at":"2099-07-19T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2099-07-19T00:00:00Z\",\"updated_at\":\"2099-07-19T00:00:00Z\",\"ttl_seconds\":10800,\"claim_id\":\"claim-generation-new\"}\n<!-- autospec-run-state:end -->"}]
JSON
fi
EOF
  chmod +x "$tmp/bin/gh"
  : > "$tmp/gh.log"

  run env PATH="$tmp/bin:$PATH" AUTOSPEC_WAIT_LOG="$tmp/gh.log" AUTOSPEC_CLAIM_RETRY_SLEEP_MS=0 \
    "$autospec" autonomous implementer-wait-failed --repo testorg/testrepo --issue 42 \
      --worker-id worker-a --branch feat/test --claim-id "$bound_claim_id" \
      --session-id session-old --diagnostic 'write_stdin failed: stdin is closed'

  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"outcome":"ownership_lost"'
  ! grep -q '^issue$' "$tmp/gh.log"
  ! grep -q 'PATCH' "$tmp/gh.log"
  rm -rf "$tmp"
}
