#!/usr/bin/env bats
setup() { script="$BATS_TEST_DIRNAME/../../scripts/autospec-self-issue.sh"; cache="$BATS_TEST_TMPDIR/cache"; }

@test "dry run uses autospec repo and records deterministic key" {
  run bash "$script" --finding '{"category":"code_health","summary":"  Broken Check  ","evidence":"log"}' --dry-run --cache "$cache"
  [ "$status" -eq 0 ]; [ "${output#*REPO: }" != "$output" ]; [ -s "$cache" ]
}

@test "repeated finding is rate limited" {
  run bash "$script" --finding '{"category":"x","summary":"same"}' --dry-run --cache "$cache"; [ "$status" -eq 0 ]
  run bash "$script" --finding '{"category":"x","summary":" SAME "}' --dry-run --cache "$cache"; [ "$status" -eq 1 ]
}

@test "missing gh fails closed" {
  run env PATH=/nonexistent /bin/bash "$script" --finding '{"category":"x","summary":"offline"}' --cache "$cache"
  [ "$status" -ne 0 ]
}
