setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-attribute.sh"
  TMP="$(mktemp -d)"
  echo '{"provider":"github","metrics":{"stars":10},"ts":1000}' > "$TMP/before.json"
  echo '{"provider":"github","metrics":{"stars":20},"ts":2000}' > "$TMP/after.json"
}
teardown() { rm -rf "$TMP"; }

@test "empty ledger -> empty table" {
  : > "$TMP/l.jsonl"; run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"
  [ "$status" -eq 0 ]; [ "$(echo "$output" | jq 'length')" = "0" ]
}
@test "single shipping lens gets full positive delta" {
  printf '%s\n' '{"issue":1,"source":"content-opportunity","outcome":"merged_clean","ts":1500}' > "$TMP/l.jsonl"
  run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.[0].source')" = "content-opportunity" ]
  [ "$(echo "$output" | jq -r '.[0].shipped')" = "1" ]
}
@test "two lenses split the delta" {
  printf '%s\n%s\n' \
    '{"issue":1,"source":"content-opportunity","outcome":"merged_clean","ts":1500}' \
    '{"issue":2,"source":"community","outcome":"published","ts":1600}' > "$TMP/l.jsonl"
  run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" = "2" ]
}
@test "malformed ledger fails closed" {
  echo 'not json' > "$TMP/l.jsonl"; run bash "$SCRIPT" "$TMP/before.json" "$TMP/after.json" "$TMP/l.jsonl"
  [ "$status" -ne 0 ]
}
