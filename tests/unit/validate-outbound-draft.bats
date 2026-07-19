setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/validate-outbound-draft.sh"
  TMP="$(mktemp -d)"
}
teardown() { rm -rf "$TMP"; }

valid_draft() {
  cat > "$TMP/d.json" <<'JSON'
{"issue":42,"platform":"reddit","target_url":"https://reddit.com/r/x","body":"Hello, useful post.","self_promo_rule":"r/x allows tool posts on Saturdays","evidence":["gsc:q=x"]}
JSON
}

@test "valid draft passes" { valid_draft; run bash "$SCRIPT" "$TMP/d.json"; [ "$status" -eq 0 ]; }
@test "empty body rejected" { valid_draft; jq '.body=""' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "missing self_promo_rule rejected" { valid_draft; jq 'del(.self_promo_rule)' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "non-int issue rejected" { valid_draft; jq '.issue="42"' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "evidence not array rejected" { valid_draft; jq '.evidence="x"' "$TMP/d.json" > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "malformed json rejected" { echo '{not json' > "$TMP/e.json"; run bash "$SCRIPT" "$TMP/e.json"; [ "$status" -ne 0 ]; }
@test "missing file rejected" { run bash "$SCRIPT" "$TMP/nope.json"; [ "$status" -ne 0 ]; }
