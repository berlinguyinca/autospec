#!/usr/bin/env bats

@test "watchdog/list-ready linked PR liveness regressions" {
    run bats tests/autospec-run/test_list_ready_issues.bats tests/unit/test_autospec_watchdog_run_state.bats
    [ "$status" -eq 0 ]
    [[ "$output" == *"issue 1877: issue 1859 is blocked while linked PR 1873 has pytest IN_PROGRESS"* ]]
    [[ "$output" == *"issue 1877: issue 1859 is not reclaimed while linked PR 1873 has pytest IN_PROGRESS"* ]]
}
