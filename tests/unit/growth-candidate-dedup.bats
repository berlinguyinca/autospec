#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
D="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-dedup.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$3\",\"roi\":3,\"effort\":\"small\",\"severity\":3,\"confidence\":0.5}"; }
ledline() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"$2\",\"norm_title\":\"$3\",\"channel\":\"content\",\"kind\":\"artifact\",\"issue\":$4,\"outcome\":\"$5\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$D" ]; run bash -n "$D"; [ "$status" -eq 0 ]
}

@test "empty ledger: all candidates pass" {
  : > "$TMP/cands.jsonl"
  cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"
  cand community "B" "b" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c '"norm_title"')" -eq 2 ]
}

@test "drops candidate matching a merged_clean ledger line" {
  bash "$LG" --append "$(ledline keyword-gap "A" "a" 7 merged_clean)"
  : > "$TMP/cands.jsonl"; cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"; cand community "B" "b" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"b"'* ]]
  [[ "$output" != *'"norm_title":"a"'* ]]
}

@test "ALSO drops candidate matching a refuted ledger line (full seen-set)" {
  bash "$LG" --append "$(ledline community "B" "b" 0 refuted)"
  : > "$TMP/cands.jsonl"; cand community "B" "b" >> "$TMP/cands.jsonl"; cand keyword-gap "A" "a" >> "$TMP/cands.jsonl"
  run bash "$D" "$TMP/cands.jsonl" "$GROWTH_LEDGER"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"a"'* ]]
  [[ "$output" != *'"norm_title":"b"'* ]]
}

@test "missing candidates file fails" {
  run bash "$D" "$TMP/nope.jsonl" "$GROWTH_LEDGER"
  [ "$status" -ne 0 ]
}
