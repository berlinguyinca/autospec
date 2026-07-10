setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-outbound-queue.sh"
  TMP="$(mktemp -d)"
  cat > "$TMP/d.json" <<'JSON'
{"issue":42,"platform":"reddit","target_url":"https://reddit.com/r/x","body":"Hello.","self_promo_rule":"Saturdays only","evidence":["gsc:q=x"]}
JSON
}
teardown() { rm -rf "$TMP"; }

@test "build-body includes target, rule, and body" {
  run bash "$SCRIPT" --build-body "$TMP/d.json"; [ "$status" -eq 0 ]
  [[ "$output" == *"https://reddit.com/r/x"* ]]
  [[ "$output" == *"Saturdays only"* ]]
  [[ "$output" == *"Hello."* ]]
}
@test "read-state approved" { run bash "$SCRIPT" --read-state "growth:outbound,growth/approved"; [ "$status" -eq 0 ]; [ "$output" = "approved" ]; }
@test "read-state rejected wins over approved" { run bash "$SCRIPT" --read-state "growth/approved,growth/rejected"; [ "$output" = "rejected" ]; }
@test "read-state edited" { run bash "$SCRIPT" --read-state "growth/edited"; [ "$output" = "edited" ]; }
@test "read-state default pending" { run bash "$SCRIPT" --read-state "growth:outbound"; [ "$output" = "pending" ]; }
@test "read-state injection-safe: crafted label does not flip state" {
  run bash "$SCRIPT" --read-state "growth/approved-not-really,foo.*"; [ "$output" = "pending" ]
}
