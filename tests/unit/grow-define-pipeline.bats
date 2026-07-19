#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
P="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-pipeline.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"$2\",\"title\":\"$3\",\"norm_title\":\"$3\",\"roi\":$4,\"effort\":\"small\",\"severity\":$5,\"confidence\":0.8}"; }
verdict() { echo "{\"norm_title\":\"$1\",\"real\":$2,\"reason\":\"$3\"}"; }
cfg() { echo "{\"grow\":{\"max_issues_per_cycle\":$1}}" > "$TMP/cfg.json"; echo "$TMP/cfg.json"; }

@test "script exists and is bash -n clean" {
  [ -f "$P" ]; run bash -n "$P"; [ "$status" -eq 0 ]
}

@test "valid+verified candidates pass; invalid dropped; refuted removed" {
  : > "$TMP/c.jsonl"
  cand keyword-gap artifact alpha 5 5 >> "$TMP/c.jsonl"
  cand community outbound beta 3 3 >> "$TMP/c.jsonl"
  echo '{"lens":"bogus-lens","kind":"artifact","title":"x","norm_title":"x"}' >> "$TMP/c.jsonl"  # invalid
  : > "$TMP/v.jsonl"
  verdict alpha true ok >> "$TMP/v.jsonl"
  verdict beta false offtopic >> "$TMP/v.jsonl"   # refuted -> removed
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"norm_title":"alpha"'* ]]
  [[ "$output" != *'"norm_title":"beta"'* ]]
  [[ "$output" != *'"norm_title":"x"'* ]]
}

@test "candidate with no verdict is fail-closed (refuted, removed)" {
  : > "$TMP/c.jsonl"; cand keyword-gap artifact gamma 4 4 >> "$TMP/c.jsonl"
  : > "$TMP/v.jsonl"   # empty verdicts
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -eq 0 ]
  [ -z "$(printf '%s' "$output" | tr -d '[:space:]')" ]
}

@test "deduped against ledger" {
  bash "$LG" --append '{"round":1,"source":"keyword-gap","title":"seen","norm_title":"seen","channel":"content","kind":"artifact","issue":9,"outcome":"merged_clean","reason":"","ts":"2026-07-09T00:00:00Z"}'
  : > "$TMP/c.jsonl"; cand keyword-gap artifact seen 5 5 >> "$TMP/c.jsonl"; cand keyword-gap artifact fresh 5 5 >> "$TMP/c.jsonl"
  : > "$TMP/v.jsonl"; verdict seen true ok >> "$TMP/v.jsonl"; verdict fresh true ok >> "$TMP/v.jsonl"
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [[ "$output" == *'"norm_title":"fresh"'* ]]
  [[ "$output" != *'"norm_title":"seen"'* ]]
}

@test "top-N slice honored" {
  : > "$TMP/c.jsonl"; : > "$TMP/v.jsonl"
  for i in 1 2 3; do cand keyword-gap artifact "t$i" 5 5 >> "$TMP/c.jsonl"; verdict "t$i" true ok >> "$TMP/v.jsonl"; done
  run bash "$P" "$TMP/c.jsonl" "$TMP/v.jsonl" "$(cfg 2)"
  [ "$(printf '%s\n' "$output" | grep -c '"norm_title"')" -eq 2 ]
}

@test "missing input file fails" {
  run bash "$P" "$TMP/nope.jsonl" "$TMP/v.jsonl" "$(cfg 8)"
  [ "$status" -ne 0 ]
}
