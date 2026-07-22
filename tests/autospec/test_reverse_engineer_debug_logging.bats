#!/usr/bin/env bats

SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/reverse-engineer.sh"

@test "reverse-engineer orchestrator avoids debug logging APIs" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$SCRIPT"
}

@test "reverse-engineer orchestrator remains shell-parseable" {
  run bash -n "$SCRIPT"
  [ "$status" -eq 0 ]
}
