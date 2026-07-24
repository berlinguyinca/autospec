#!/usr/bin/env bats

@test "force restart targets the detached conductor process group" {
  grep -q 'kill -- "-\$_group_pid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous.sh"
}

@test "force restart retains a single-process fallback" {
  grep -q 'kill "\$_group_pid"' \
    "$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous.sh"
}
