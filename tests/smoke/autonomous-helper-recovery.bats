#!/usr/bin/env bats

@test "autonomous timeline helper recovery regression passes" {
  run env -u AUTOSPEC_STOP_FLAG_FILE cargo test -p autospec-cli --test cli_commands autonomous_timeline_rate_limits_leader_nudges_and_reports_helper_recovery
  [ "$status" -eq 0 ]
}
