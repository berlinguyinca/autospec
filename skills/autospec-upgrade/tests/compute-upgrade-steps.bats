#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../.." && pwd)"
  STEPS="$REPO_ROOT/skills/autospec-upgrade/scripts/compute-upgrade-steps.sh"
  FX="$BATS_TEST_DIRNAME/fixtures/compute-steps"
}

@test "compute-upgrade-steps.sh exists and is executable" {
  [ -x "$STEPS" ]
}

@test "Angular 20 -> target 23 yields strictly sequential hops 21,22,23 (no skips)" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-angular20.json" --targets "$FX/targets.json"
  [ "$status" -eq 0 ]
  # The to-sequence must be exactly [21,22,23] — never skipping a major.
  echo "$output" | jq -e '[.hops[] | select(.framework=="angular") | .to] == [21,22,23]'
  # And each hop's from is the prior major (contiguous chain).
  echo "$output" | jq -e '[.hops[] | select(.framework=="angular") | .from] == [20,21,22]'
}

@test "already-latest Angular yields an empty hop list" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-angular-latest.json" --targets "$FX/targets.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.hops == []'
}

@test "Next 13 -> 15 yields incremental one-major hops 14,15" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-next13.json" --targets "$FX/targets.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.hops[] | select(.framework=="next") | .to] == [14,15]'
  # detect-next13 also has react@18 -> 19, one hop.
  echo "$output" | jq -e '[.hops[] | select(.framework=="react") | .to] == [19]'
}

@test "React 17 -> 19 yields incremental one-major hops 18,19" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-react17.json" --targets "$FX/targets.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.hops[] | select(.framework=="react") | .to] == [18,19]'
}

@test "combined angular+react stack yields strictly incremental hops for both" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-combo.json" --targets "$FX/targets.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.hops[] | select(.framework=="angular") | .to] == [22,23]'
  echo "$output" | jq -e '[.hops[] | select(.framework=="react") | .to] == [19]'
}

@test "--target-<fw> flag overrides the targets file" {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
  run bash "$STEPS" --detect "$FX/detect-angular20.json" --target-angular 22
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.hops[] | select(.framework=="angular") | .to] == [21,22]'
}

@test "missing --detect exits non-zero" {
  run bash "$STEPS" --targets "$FX/targets.json"
  [ "$status" -ne 0 ]
}
