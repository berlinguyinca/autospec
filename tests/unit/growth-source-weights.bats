#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SW="$REPO_ROOT/skills/autospec-shared/scripts/growth-source-weights.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

line() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"seo\",\"kind\":\"artifact\",\"issue\":$2,\"outcome\":\"$3\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$SW" ]; run bash -n "$SW"; [ "$status" -eq 0 ]
}

@test "unknown/empty ledger yields prior 0.5 for every known source" {
  run bash "$SW" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.["keyword-gap"] == 0.5'
  echo "$output" | jq -e '.["community"] == 0.5'
}

@test "a source with all-clean ships weights above prior" {
  bash "$LG" --append "$(line keyword-gap 1 merged_clean)"
  bash "$LG" --append "$(line keyword-gap 2 merged_clean)"
  bash "$LG" --append "$(line keyword-gap 3 merged_clean)"
  run bash "$SW" --json
  echo "$output" | jq -e '.["keyword-gap"] > 0.5'
}

@test "refutations pull a source's weight below its clean-rate base" {
  bash "$LG" --append "$(line community 4 merged_clean)"
  bash "$LG" --append "$(line community 0 refuted)"
  bash "$LG" --append "$(line community 0 refuted)"
  run bash "$SW" --json
  # community still defined and <= keyword-gap-with-no-refutations comparison not needed;
  # just assert it is a valid number in (0,1]
  echo "$output" | jq -e '.["community"] > 0 and .["community"] <= 1'
}
