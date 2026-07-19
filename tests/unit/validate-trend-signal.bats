#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-trend-signal.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid() {
  cat > "$TMP/s.json" <<'JSON'
{"source":"internet-forums","kind":"pain-point","summary":"users hit rate limit",
 "norm_key":"users hit rate limit","evidence_ref":"https://example.com/thread/1",
 "first_seen":"2026-07-01T00:00:00Z","recurrence":2,
 "sanitized_excerpt":"users report hitting the rate limit often",
 "ts":"2026-07-08T00:00:00Z"}
JSON
  echo "$TMP/s.json"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a valid trend signal" {
  run bash "$V" "$(valid)"; [ "$status" -eq 0 ]
}

@test "rejects missing source" {
  f="$(valid)"; jq 'del(.source)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"source"* ]]
}

@test "rejects missing kind" {
  f="$(valid)"; jq 'del(.kind)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"kind"* ]]
}

@test "rejects missing summary" {
  f="$(valid)"; jq 'del(.summary)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"summary"* ]]
}

@test "rejects missing norm_key" {
  f="$(valid)"; jq 'del(.norm_key)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"norm_key"* ]]
}

@test "rejects missing evidence_ref" {
  f="$(valid)"; jq 'del(.evidence_ref)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"evidence_ref"* ]]
}

@test "rejects missing first_seen" {
  f="$(valid)"; jq 'del(.first_seen)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"first_seen"* ]]
}

@test "rejects missing recurrence" {
  f="$(valid)"; jq 'del(.recurrence)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"recurrence"* ]]
}

@test "rejects missing sanitized_excerpt" {
  f="$(valid)"; jq 'del(.sanitized_excerpt)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"sanitized_excerpt"* ]]
}

@test "rejects missing ts" {
  f="$(valid)"; jq 'del(.ts)' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"ts"* ]]
}

@test "rejects non-integer recurrence" {
  f="$(valid)"; jq '.recurrence=2.5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"recurrence"* ]]
}

@test "rejects string recurrence" {
  f="$(valid)"; jq '.recurrence="2"' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]
}

@test "rejects non-string norm_key" {
  f="$(valid)"; jq '.norm_key=5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"norm_key"* ]]
}

@test "rejects non-string summary" {
  f="$(valid)"; jq '.summary=5' "$f" > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"summary"* ]]
}

@test "rejects non-object JSON" {
  printf '[1,2,3]' > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -ne 0 ]; [[ "$output" == *"object"* ]]
}

@test "rejects malformed json with exit 2" {
  printf 'not json {{{' > "$TMP/b.json"
  run bash "$V" "$TMP/b.json"; [ "$status" -eq 2 ]
}

@test "rejects empty object via stdin path with nonzero exit" {
  printf '{}' > "$TMP/empty.json"
  run bash "$V" "$TMP/empty.json"; [ "$status" -ne 0 ]
}

@test "works via /dev/stdin piping" {
  run bash -c "printf '{}' | bash '$V' /dev/stdin"
  [ "$status" -ne 0 ]
}
