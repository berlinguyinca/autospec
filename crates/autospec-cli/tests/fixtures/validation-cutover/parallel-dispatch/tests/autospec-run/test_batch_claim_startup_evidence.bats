#!/usr/bin/env bats

@test "startup evidence fixture" {
  [[ -z "${AUTOSPEC_RUN_ONLY_ISSUES:-}" ]]
}
