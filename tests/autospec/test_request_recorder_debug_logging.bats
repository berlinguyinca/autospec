#!/usr/bin/env bats

RECORDER="$BATS_TEST_DIRNAME/../../skills/autospec-test/scripts/window-contract/request-recorder.mjs"

@test "request recorder contains no debug logging APIs" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$RECORDER"
}
