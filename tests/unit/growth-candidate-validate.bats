#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-growth-candidate.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid() {
  cat > "$TMP/c.json" <<'JSON'
{"lens":"keyword-gap","channel":"content","kind":"artifact",
 "title":"Add vs page","norm_title":"add vs page",
 "rationale":"gsc pos 12","evidence":["gsc:q=x vs y"],
 "roi":4,"effort":"medium","severity":3,"confidence":0.7}
JSON
  echo "$TMP/c.json"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a valid candidate" {
  run bash "$V" "$(valid)"; [ "$status" -eq 0 ]
}

@test "rejects unknown lens" {
  f="$(valid)"; jq '.lens="seo-wizard"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"lens"* ]]
}

@test "rejects roi out of range" {
  f="$(valid)"; jq '.roi=9' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"roi"* ]]
}

@test "rejects non-integer severity" {
  f="$(valid)"; jq '.severity=2.5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects bad effort" {
  f="$(valid)"; jq '.effort="huge"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"effort"* ]]
}

@test "rejects bad kind" {
  f="$(valid)"; jq '.kind="tweet"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"kind"* ]]
}

@test "rejects confidence above 1" {
  f="$(valid)"; jq '.confidence=1.5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects missing norm_title" {
  f="$(valid)"; jq 'del(.norm_title)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects malformed json" {
  printf 'not json {{{' > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects numeric title" {
  f="$(valid)"; jq '.title=5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"title"* ]]
}

@test "rejects array norm_title" {
  f="$(valid)"; jq '.norm_title=["x"]' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"norm_title"* ]]
}

@test "rejects non-object JSON" {
  printf '[1,2,3]' > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"object"* ]]
}
