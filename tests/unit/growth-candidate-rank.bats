#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
R="$REPO_ROOT/skills/autospec-shared/scripts/growth-candidate-rank.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

cand() { echo "{\"lens\":\"$1\",\"channel\":\"content\",\"kind\":\"artifact\",\"title\":\"$2\",\"norm_title\":\"$2\",\"roi\":$3,\"effort\":\"$4\",\"severity\":$5,\"confidence\":$6}"; }

@test "script exists and is bash -n clean" {
  [ -f "$R" ]; run bash -n "$R"; [ "$status" -eq 0 ]
}

@test "higher roi/severity ranks first (empty ledger → equal source weights)" {
  : > "$TMP/c.jsonl"
  cand keyword-gap low 2 small 2 0.5 >> "$TMP/c.jsonl"
  cand keyword-gap high 5 small 5 0.9 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  [ "$status" -eq 0 ]
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"high"'* ]]
}

@test "effort_factor lowers a large-effort candidate below an equal small-effort one" {
  : > "$TMP/c.jsonl"
  cand keyword-gap big 4 large 4 0.8 >> "$TMP/c.jsonl"
  cand keyword-gap small 4 small 4 0.8 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"small"'* ]]
}

@test "severity-first tiebreak on equal rank_score" {
  # identical roi/effort/confidence/lens → same base; differ only in severity via equal score construction:
  # give equal rank_score by same fields but different severity+roi that produce equal (roi+severity) sum.
  : > "$TMP/c.jsonl"
  cand keyword-gap sevhi 2 small 5 0.5 >> "$TMP/c.jsonl"   # roi2 sev5 sum7
  cand keyword-gap sevlo 5 small 2 0.5 >> "$TMP/c.jsonl"   # roi5 sev2 sum7  → equal rank_score
  run bash "$R" "$TMP/c.jsonl"
  first="$(printf '%s\n' "$output" | head -1)"
  [[ "$first" == *'"title":"sevhi"'* ]]
}

@test "attaches numeric rank_score" {
  : > "$TMP/c.jsonl"; cand keyword-gap a 3 small 3 0.6 >> "$TMP/c.jsonl"
  run bash "$R" "$TMP/c.jsonl"
  echo "$output" | jq -e '.rank_score | numbers' >/dev/null
}
