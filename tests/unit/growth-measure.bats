#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
M="$REPO_ROOT/skills/autospec-shared/scripts/growth-measure.sh"
FIX="$REPO_ROOT/tests/fixtures/growth/gsc-sample.json"

setup() { TMP="$(mktemp -d)"; export GROWTH_NOW_EPOCH=1000000; }
teardown() { rm -rf "$TMP"; unset GROWTH_NOW_EPOCH; }

@test "script exists and is bash -n clean" {
  [ -f "$M" ]; run bash -n "$M"; [ "$status" -eq 0 ]
}

@test "gsc normalize sums clicks and impressions" {
  run bash "$M" --normalize gsc "$FIX"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.provider == "gsc"'
  echo "$output" | jq -e '.metrics.clicks_total == 15'
  echo "$output" | jq -e '.metrics.impressions_total == 550'
  echo "$output" | jq -e '.ts == 1000000'
}

@test "github normalize maps stars and forks" {
  echo '{"stargazers_count":42,"forks_count":5}' > "$TMP/gh.json"
  run bash "$M" --normalize github "$TMP/gh.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.metrics.stars == 42'
  echo "$output" | jq -e '.metrics.forks == 5'
}

@test "unknown provider fails" {
  echo '{}' > "$TMP/x.json"
  run bash "$M" --normalize wat "$TMP/x.json"
  [ "$status" -ne 0 ]
}
