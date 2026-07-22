#!/usr/bin/env bats

@test "calculator fixture emits no debug logging" {
    local fixture="$BATS_TEST_DIRNAME/../fixtures/lang/js/tests/calculator.test.js"
    ! grep -Eq '(^|[^[:alnum:]_])console\.(log|debug|info|warn|error)[[:space:]]*\(' "$fixture"
}
