#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
S="$REPO_ROOT/skills/autospec-shared/scripts"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$2\",\"roi\":$3,\"effort\":\"small\",\"severity\":$4,\"confidence\":0.8}"; }
ledline() { echo "{\"round\":1,\"source\":\"keyword-gap\",\"title\":\"seen\",\"norm_title\":\"seen\",\"channel\":\"content\",\"kind\":\"artifact\",\"issue\":5,\"outcome\":\"merged_clean\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "validate -> dedup -> verify -> rank end-to-end" {
  # a previously-shipped item is in the ledger and must be deduped out
  bash "$S/growth-ledger.sh" --append "$(ledline)"

  : > "$TMP/cands.jsonl"
  cand keyword-gap seen 5 5 >> "$TMP/cands.jsonl"   # duplicate of ledger -> dropped
  cand keyword-gap alpha 5 5 >> "$TMP/cands.jsonl"  # strong
  cand community beta 2 2 >> "$TMP/cands.jsonl"     # weak

  # 1. validate each candidate
  while IFS= read -r line; do
    echo "$line" > "$TMP/one.json"
    bash "$S/validate-growth-candidate.sh" "$TMP/one.json"
  done < "$TMP/cands.jsonl"

  # 2. dedup against ledger
  bash "$S/growth-candidate-dedup.sh" "$TMP/cands.jsonl" "$GROWTH_LEDGER" > "$TMP/deduped.jsonl"
  [ "$(grep -c '"norm_title"' "$TMP/deduped.jsonl")" -eq 2 ]
  ! grep -q '"norm_title":"seen"' "$TMP/deduped.jsonl"

  # 3. verify each survivor (all real:true here) -> collect
  echo '{"real":true,"reason":"ok"}' > "$TMP/v.json"
  : > "$TMP/verified.jsonl"
  while IFS= read -r line; do
    echo "$line" > "$TMP/one.json"
    bash "$S/growth-candidate-verify.sh" "$TMP/one.json" "$TMP/v.json" >> "$TMP/verified.jsonl"
  done < "$TMP/deduped.jsonl"

  # 4. rank -> alpha (roi5/sev5) must be first, beta last
  run bash "$S/growth-candidate-rank.sh" "$TMP/verified.jsonl"
  [ "$status" -eq 0 ]
  first="$(printf '%s\n' "$output" | head -1)"
  last="$(printf '%s\n' "$output" | tail -1)"
  [[ "$first" == *'"title":"alpha"'* ]]
  [[ "$last" == *'"title":"beta"'* ]]
}
