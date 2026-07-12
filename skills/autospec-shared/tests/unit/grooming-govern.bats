#!/usr/bin/env bats
# Tests for grooming-govern.sh — telemetry-driven self-governance of the
# backlog-grooming active-gate set. Mirrors advisor-govern.sh: promotes
# eligible-promote -> template-promote when the groomed clean-merge rate is
# >= baseline over a minimum-sample floor; retracts on regression; never
# retracts below the seed (eligible-promote).

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/grooming-govern.sh"
  TMP="$(mktemp -d)"
  export AUTOSPEC_GROOMING_STATE_DIR="$TMP/state"
}

teardown() { rm -rf "$TMP"; }

@test "seed active set is eligible-promote only" {
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote"]' >/dev/null
}

@test "promotes template-promote when groomed>=baseline over floor" {
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":25,"baseline_samples":25}' --min-samples 20
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote","template-promote"]' >/dev/null
}

@test "holds below sample floor" {
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":5,"baseline_samples":5}' --min-samples 20
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote"]' >/dev/null
}

@test "does NOT widen on empty/zero baseline (widen-guard, self-governance)" {
  # Reproduces the defect: the loop wrote only groomed records so observe yields
  # groomed_rate=1.0, baseline_rate=0.0, baseline_samples=0. Promoting on
  # 1.0 >= 0.0 would enable template-promote on ZERO real quality signal.
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":1.0,"baseline_clean_merge_rate":0,"samples":25,"baseline_samples":0}' --min-samples 20
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote"]' >/dev/null
}

@test "widens when baseline_samples meets the floor and groomed>=baseline" {
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":25,"baseline_samples":25}' --min-samples 20
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote","template-promote"]' >/dev/null
}

@test "tick reports hold action when baseline signal is insufficient" {
  run bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":1.0,"baseline_clean_merge_rate":0,"samples":25,"baseline_samples":0}' --min-samples 20
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.action == "hold"' >/dev/null
  echo "$output" | jq -e '.active == ["eligible-promote"]' >/dev/null
}

@test "retracts on regression, never below seed" {
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":0.9,"baseline_clean_merge_rate":0.8,"samples":25,"baseline_samples":25}' --min-samples 20
  bash "$SCRIPT" tick --observed '{"groomed_clean_merge_rate":0.5,"baseline_clean_merge_rate":0.8,"samples":25,"baseline_samples":25}' --min-samples 20
  run bash "$SCRIPT" show
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.active == ["eligible-promote"]' >/dev/null
}
