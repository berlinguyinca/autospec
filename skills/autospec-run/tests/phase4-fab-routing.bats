#!/usr/bin/env bats
# skills/autospec-run/tests/phase4-fab-routing.bats — TDD for fab-route.sh (issue #1235)
#
# Verifies Phase 4 label-based routing: `area:fab` / `autospec:fab-flow` issues
# route to the fab implementer gate; every other issue keeps the default gate.
# Real label parsing via the helper — no mocks.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/fab-route.sh"

@test "fab-route.sh exists and is executable" {
  [ -f "$SCRIPT" ]
}

@test "area:fab label routes to the fab gate" {
  run bash "$SCRIPT" --labels "auto-implement,area:fab"
  [ "$status" -eq 0 ]
  [ "$output" = "fab" ]
}

@test "autospec:fab-flow label routes to the fab gate" {
  run bash "$SCRIPT" --labels "auto-implement,autospec:fab-flow,ctx:64k"
  [ "$status" -eq 0 ]
  [ "$output" = "fab" ]
}

@test "non-fab issue keeps the default gate" {
  run bash "$SCRIPT" --labels "auto-implement,ctx:64k,reasoning:medium"
  [ "$status" -eq 0 ]
  [ "$output" = "default" ]
}

@test "empty labels keep the default gate" {
  run bash "$SCRIPT" --labels ""
  [ "$status" -eq 0 ]
  [ "$output" = "default" ]
}

@test "labels read from stdin route correctly" {
  run bash -c 'printf "%s\n" "auto-implement,area:fab" | "'"$SCRIPT"'" --stdin'
  [ "$status" -eq 0 ]
  [ "$output" = "fab" ]
}

@test "a label that merely contains 'fab' as a substring does not misroute" {
  run bash "$SCRIPT" --labels "auto-implement,area:fabric,prefab"
  [ "$status" -eq 0 ]
  [ "$output" = "default" ]
}

@test "whitespace around labels is tolerated" {
  run bash "$SCRIPT" --labels " auto-implement , area:fab "
  [ "$status" -eq 0 ]
  [ "$output" = "fab" ]
}

@test "growth:artifact routes to growth" {
  run bash "$SCRIPT" --labels "auto-implement,growth:artifact,growth:content"
  [ "$status" -eq 0 ]
  [ "$output" = "growth" ]
}

@test "fab wins over growth when both present" {
  run bash "$SCRIPT" --labels "growth:artifact,area:fab"
  [ "$output" = "fab" ]
}

@test "growth:artifactx does not route to growth (whole-label)" {
  run bash "$SCRIPT" --labels "growth:artifactx"
  [ "$output" = "default" ]
}

@test "plain auto-implement still default" {
  run bash "$SCRIPT" --labels "auto-implement,ctx:64k"
  [ "$output" = "default" ]
}
