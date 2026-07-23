#!/usr/bin/env bats

@test "preview fallback emits ranked candidates without filing" {
  run bash -c 'AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="printf {\\\"proposals_total\\\":0,\\\"proposals\\\":[]}" bash scripts/autospec-explore.sh --once --preview --autonomous --research-sources spec-vs-code'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"filed":0'* ]]
  [[ "$output" == *'"new_candidates":'* ]]
}

# Keep the preview contract covered when the research-cycle adapter changes.
