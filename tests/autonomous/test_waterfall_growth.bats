#!/usr/bin/env bats
# tests/autonomous/test_waterfall_growth.bats — capability-gated GROWTH tiers
# appended after Tier 4 (before idle-rescan) in scripts/autonomous-waterfall.sh.

setup() { SCRIPT="$BATS_TEST_DIRNAME/../../scripts/autonomous-waterfall.sh"; }

# Force all code tiers dry so the cascade reaches the growth section.
DRY="--dry-cycles 9 --tier15-dry-cycles 9 --tier2-dry-cycles 9 --tier3-dry-cycles 9 --tier4-dry-cycles 9 --backlog-count 0 --open-issue-count 0"

@test "growth disabled: never emits a growth action (regression)" {
  run bash "$SCRIPT" $DRY
  [ "$status" -eq 0 ]
  [[ "$output" != *growth* ]]
  [[ "$output" == *idle-rescan* ]]
}

@test "outbound pending -> service-growth-outbound (tier 5)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 2
  [[ "$output" == *service-growth-outbound* ]]
  [[ "$output" == *'"tier":5'* ]]
}

@test "backlog below floor -> run-growth-define (tier 6)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 1 --growth-backlog-floor 3
  [[ "$output" == *run-growth-define* ]]
  [[ "$output" == *'"tier":6'* ]]
}

@test "measure due -> run-growth-measure (tier 7)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 5 --growth-backlog-floor 3 --growth-measure-due 1
  [[ "$output" == *run-growth-measure* ]]
  [[ "$output" == *'"tier":7'* ]]
}

@test "outbound outranks define" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 1 --growth-backlog 0 --growth-backlog-floor 3
  [[ "$output" == *service-growth-outbound* ]]
}

@test "no growth work -> idle-rescan even when enabled" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 5 --growth-backlog-floor 3 --growth-measure-due 0
  [[ "$output" == *idle-rescan* ]]
}

@test "growth does not preempt a non-empty code backlog" {
  run bash "$SCRIPT" --backlog-count 3 --growth-enabled 1 --growth-outbound-pending 5
  [[ "$output" == *'"tier":1'* ]]
  [[ "$output" != *growth* ]]
}
