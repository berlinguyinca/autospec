#!/usr/bin/env bats
setup() { script="$BATS_TEST_DIRNAME/../scripts/explore-research-cycle.sh"; schema="$BATS_TEST_DIRNAME/../schemas/autospec-explore-proposal.schema.json"; }
@test "classifier emits repo class on aggregate and proposals" {
  run grep -F 'repo_class' "$script"; [ "$status" -eq 0 ]
}
@test "classifier recognizes governance-only repository classes" {
  run grep -E "archived|data-study|infra|docs" "$script"; [ "$status" -eq 0 ]
}
@test "proposal schema enumerates repo classes" {
  run grep -F 'repo_class' "$schema"; [ "$status" -eq 0 ]
}
