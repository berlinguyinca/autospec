#!/usr/bin/env bats

@test "explore drain isolates and kills the harness process group via the shared helper" {
  drain="$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-explore-drain.sh"
  grep -q 'run_in_new_session "\$HARNESS_DISPATCHER" exec' "$drain"
  grep -q 'lib/autospec-process-tree.sh' "$drain"
  grep -q 'autospec_kill_tree "\$child_pid" separate' "$drain"
  grep -q 'AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 120' "$drain"
  # The drain delegates to the shared lib; no local kill-tree definition remains.
  ! grep -qE '^[[:space:]]*kill_tree[[:space:]]*\(\)' "$drain"
}
