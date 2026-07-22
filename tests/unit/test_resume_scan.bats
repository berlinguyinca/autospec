#!/usr/bin/env bats

SCAN="$BATS_TEST_DIRNAME/../../skills/autospec-resume/scripts/resume-scan.sh"

@test "resume scanner contains no explicit any usage" {
    run grep -En '(^|[^[:alnum:]_])any([^[:alnum:]_]|$)' "$SCAN"
    [ "$status" -eq 1 ]
}
