#!/usr/bin/env bats
# tests/lint-factual-claims.bats
#
# Exercises scripts/lint-factual-claims.sh against populated positive and
# negative cases (issue #3870): bare externally checkable assertions in
# comments must fail; dated+sourced, runtime-checked, and waived claims must
# pass.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-factual-claims.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-factual-claims"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"FACTUAL_CLAIM"* ]]
}

@test "bare registry-visibility assertion is flagged" {
  run bash "$LINT" "$FIX/pos1-registry-bare.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"FACTUAL_CLAIM:"* ]]
  [[ "$output" == *"REGISTRY_VISIBILITY"* ]]
  [[ "$output" == *"pos1-registry-bare.sh"* ]]
}

@test "bare host-capability assertion is flagged" {
  run bash "$LINT" "$FIX/pos2-host-bare.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"HOST_CAPABILITY"* ]]
  [[ "$output" == *"pos2-host-bare.sh"* ]]
}

@test "bare network-reachability assertion in a workflow is flagged" {
  run bash "$LINT" "$FIX/pos3-network-bare.yml"
  [ "$status" -eq 1 ]
  [[ "$output" == *"NETWORK_REACHABILITY"* ]]
  [[ "$output" == *"pos3-network-bare.yml"* ]]
}

@test "dated and sourced claim passes with no runtime check" {
  run bash "$LINT" "$FIX/neg1-dated-sourced.sh"
  [ "$status" -eq 0 ]
}

@test "bare claim passes when the same file checks registry visibility at runtime" {
  run bash "$LINT" "$FIX/neg2-runtime-registry.sh"
  [ "$status" -eq 0 ]
}

@test "bare claim passes when the same file probes the host capability at runtime" {
  run bash "$LINT" "$FIX/neg3-runtime-host.sh"
  [ "$status" -eq 0 ]
}

@test "waived claim passes and emits an advisory INFO line" {
  run bash "$LINT" "$FIX/neg4-opt-out.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:FACTUAL_CLAIM:"* ]]
  [[ "$output" == *"opt-out"* ]]
}

@test "bare linter:allow-FACTUAL_CLAIM without a reason is rejected" {
  run bash "$LINT" "$FIX/neg5-bare-opt-out.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"FACTUAL_CLAIM:"* ]]
}

@test "intent comment is not a finding" {
  run bash "$LINT" "$FIX/neg6-intent.sh"
  [ "$status" -eq 0 ]
}

@test "dated workflow comment passes even though YAML carries no runtime check" {
  run bash "$LINT" "$FIX/neg7-dated-workflow.yml"
  [ "$status" -eq 0 ]
}

@test "directory scan counts every bare assertion in the exit code" {
  run bash "$LINT" "$FIX"
  [ "$status" -eq 4 ]
  [ "$(printf '%s\n' "$output" | grep -c '^FACTUAL_CLAIM:')" -eq 4 ]
}

@test "default scan of scripts/ and .github/workflows/ passes on this tree" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
}
