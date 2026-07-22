#!/usr/bin/env bats

@test "greeter fixture emits no debug logging" {
    local fixture="$BATS_TEST_DIRNAME/../fixtures/lang/js/tests/greeter.test.ts"
    ! grep -Eq '(^|[^[:alnum:]_])console\.(log|debug|info|warn|error)[[:space:]]*\(' "$fixture"
}
