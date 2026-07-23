#!/usr/bin/env bats

setup() { script="$BATS_TEST_DIRNAME/../scripts/explore-research-cycle.sh"; }

@test "restricted classes expose the safety gate" {
  run grep -E 'data-study|notebook-research|preservation_rollback_plan' "$script"
  [ "$status" -eq 0 ]
}

@test "restricted mode rejects rewrite proposals" {
  run grep -E 'rewrite|reformat|generated-data|rejected' "$script"
  [ "$status" -eq 0 ]
}
