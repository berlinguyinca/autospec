#!/usr/bin/env bats

FIXTURE="$BATS_TEST_DIRNAME/../fixtures/autonomy-v2/node-cli-tool/bin/fixture.js"

@test "node CLI fixture avoids debug logging APIs" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|debugger' "$FIXTURE"
}

@test "node CLI fixture preserves its stdout contract" {
  run node "$FIXTURE"
  [ "$status" -eq 0 ]
  [ "$output" = "fixture" ]
}
