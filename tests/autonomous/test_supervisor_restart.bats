#!/usr/bin/env bats

setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous.sh"
}

@test "supervisor has a restart path for a stopped conductor" {
  run grep -F 'start_detached' "$SCRIPT"
  [ "$status" -eq 0 ]
  run grep -F 'restarted stopped conductor' "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "supervisor gates restart on the stop sentinel" {
  run grep -F '[ ! -f "$STOP_FLAG_FILE" ]' "$SCRIPT"
  [ "$status" -eq 0 ]
}
