#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
IP="$REPO_ROOT/skills/autospec-shared/scripts/discovery-intersect-prefilter.sh"
TL="$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh"

setup() { TMP="$(mktemp -d)"; export AUTOSPEC_TREND_LEDGER="$TMP/ledger.jsonl"; unset AUTOSPEC_TREND_MIN_RECURRENCE; }
teardown() { rm -rf "$TMP"; unset AUTOSPEC_TREND_LEDGER; unset AUTOSPEC_TREND_MIN_RECURRENCE; }

sig() {
  # sig <source> <norm_key> <recurrence>
  echo "{\"source\":\"$1\",\"kind\":\"complaint\",\"summary\":\"s\",\"norm_key\":\"$2\",\"evidence_ref\":\"https://example.com\",\"first_seen\":\"2026-07-08T00:00:00Z\",\"recurrence\":$3,\"sanitized_excerpt\":\"e\",\"ts\":\"2026-07-08T00:00:00Z\"}"
}

@test "script exists and is bash -n clean" {
  [ -f "$IP" ]; run bash -n "$IP"; [ "$status" -eq 0 ]
}

@test "absent ledger emits [] with exit 0" {
  run bash "$IP"
  [ "$status" -eq 0 ]
  [ "$output" = "[]" ]
}

@test "empty ledger emits [] with exit 0" {
  : > "$AUTOSPEC_TREND_LEDGER"
  run bash "$IP"
  [ "$status" -eq 0 ]
  [ "$output" = "[]" ]
}

@test "default min=2 drops recurrence-1 signals, keeps recurrence>=2" {
  bash "$TL" --append "$(sig internet-forums below-min 1)"
  bash "$TL" --append "$(sig internet-forums at-min 2)"
  run bash "$IP"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="below-min")] | length')" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="at-min")] | length')" -eq 1 ]
}

@test "--min N overrides the default threshold" {
  bash "$TL" --append "$(sig internet-forums two 2)"
  bash "$TL" --append "$(sig internet-forums three 3)"
  run bash "$IP" --min 3
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="two")] | length')" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="three")] | length')" -eq 1 ]
}

@test "AUTOSPEC_TREND_MIN_RECURRENCE env var sets the default threshold" {
  bash "$TL" --append "$(sig internet-forums two 2)"
  bash "$TL" --append "$(sig internet-forums four 4)"
  AUTOSPEC_TREND_MIN_RECURRENCE=4 run bash "$IP"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="two")] | length')" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="four")] | length')" -eq 1 ]
}

@test "output is sorted by recurrence descending" {
  bash "$TL" --append "$(sig internet-forums low 2)"
  bash "$TL" --append "$(sig internet-forums high 9)"
  bash "$TL" --append "$(sig internet-forums mid 5)"
  run bash "$IP"
  [ "$status" -eq 0 ]
  first="$(echo "$output" | jq -r '.[0].norm_key')"
  second="$(echo "$output" | jq -r '.[1].norm_key')"
  third="$(echo "$output" | jq -r '.[2].norm_key')"
  [ "$first" = "high" ]
  [ "$second" = "mid" ]
  [ "$third" = "low" ]
}

@test "output is a compact single-line JSON array" {
  bash "$TL" --append "$(sig internet-forums one 2)"
  run bash "$IP"
  [ "$status" -eq 0 ]
  lines="$(printf '%s\n' "$output" | wc -l | tr -d ' ')"
  [ "$lines" -eq 1 ]
  echo "$output" | jq -e 'type == "array"' >/dev/null
}
