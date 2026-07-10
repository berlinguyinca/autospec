bats_require_minimum_version 1.5.0

# Drives the deterministic spine end-to-end with mock gh / fetch. Subprocess
# mocks (not live subagents) prevent the monitor-stall failure mode.
setup() {
  ROOT="$BATS_TEST_DIRNAME/../.."
  DIR="$ROOT/skills/autospec-shared/scripts"
  TMP="$(mktemp -d)"
  export GROWTH_NOW_EPOCH=1000
  export GROWTH_LEDGER="$TMP/ledger.jsonl"; : > "$GROWTH_LEDGER"
}
teardown() { rm -rf "$TMP"; }

@test "valid draft is queued; malformed draft is dropped" {
  cat > "$TMP/good.json" <<'JSON'
{"issue":1,"platform":"reddit","target_url":"https://r/x","body":"Hi.","self_promo_rule":"ok","evidence":[]}
JSON
  run bash "$DIR/validate-outbound-draft.sh" "$TMP/good.json"; [ "$status" -eq 0 ]
  run bash "$DIR/growth-outbound-queue.sh" --build-body "$TMP/good.json"; [ "$status" -eq 0 ]
  [[ "$output" == *"https://r/x"* ]]
  echo '{"body":""}' > "$TMP/bad.json"
  run bash "$DIR/validate-outbound-draft.sh" "$TMP/bad.json"; [ "$status" -ne 0 ]
}

@test "over-cadence draft is refused" {
  echo '{"approval":{"cadence_caps":{"default_per_platform_per_week":1}}}' > "$TMP/cfg.json"
  printf '%s\n' '{"issue":9,"source":"community","platform":"reddit","outcome":"published","ts":1000}' > "$GROWTH_LEDGER"
  run bash "$DIR/growth-ethics-precheck.sh" --cadence "$TMP/cfg.json" "$GROWTH_LEDGER" reddit
  [ "$status" -ne 0 ]
}

@test "approved state produces package path but never posts (no gh in helper)" {
  run bash "$DIR/growth-outbound-queue.sh" --read-state "growth/approved"; [ "$output" = "approved" ]
}

@test "measure degrades to fail-closed on missing creds" {
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"DEFINITELY_UNSET_TOK"}}}' > "$TMP/cfg.json"
  # --separate-stderr: fail-closed must be LOUD (a reason on stderr), not a
  # silent exit — stdout (the envelope) must stay empty.
  run --separate-stderr bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"
  [ "$status" -ne 0 ]; [ -z "$output" ]; [ -n "$stderr" ]
}
