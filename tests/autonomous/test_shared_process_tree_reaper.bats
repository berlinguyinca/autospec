#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  REAPER="$REPO_ROOT/scripts/lib/process-tree.sh"
  spawned_pids=""
}

teardown() {
  for pid in $spawned_pids; do
    if ! kill "$pid" 2>/dev/null; then
      :
    fi
    if ! kill -9 "$pid" 2>/dev/null; then
      :
    fi
  done
}

@test "autonomous process reapers source one shared implementation" {
  for script in \
    scripts/autospec-autonomous-explore-drain.sh \
    scripts/autospec-autonomous-verify-drain.sh \
    scripts/autospec-autonomous-run-drain.sh \
    scripts/autospec-explore.sh; do
    grep -q 'lib/process-tree.sh' "$REPO_ROOT/$script"
    ! grep -Eq '^(_explore_)?kill_tree\(\)' "$REPO_ROOT/$script"
  done
}

@test "shared reaper terminates a detached process group and its child" {
  child_file="$BATS_TEST_TMPDIR/detached-child.pid"
  setsid bash -c 'sleep 300 & printf "%s\n" "$!" > "$1"; wait' _ "$child_file" &
  leader_pid="$!"
  spawned_pids="$leader_pid"

  for _ in $(seq 1 50); do
    [ -s "$child_file" ] && break
    sleep 0.02
  done
  [ -s "$child_file" ]
  child_pid="$(cat "$child_file")"
  spawned_pids="$spawned_pids $child_pid"

  # shellcheck source=/dev/null
  . "$REAPER"
  autospec_kill_process_tree "$leader_pid" 0
  if ! wait "$leader_pid" 2>/dev/null; then
    :
  fi

  run kill -0 "$leader_pid"
  [ "$status" -ne 0 ]
  run bash -c 'state="$(ps -o stat= -p "$1" 2>/dev/null)"; [[ -z "$state" || "$state" == Z* ]]' _ "$child_pid"
  [ "$status" -eq 0 ]
}

@test "shared reaper recursively terminates a process tree in the caller group" {
  child_file="$BATS_TEST_TMPDIR/child.pid"
  bash -c 'sleep 300 & printf "%s\n" "$!" > "$1"; wait' _ "$child_file" &
  parent_pid="$!"
  spawned_pids="$parent_pid"

  for _ in $(seq 1 50); do
    [ -s "$child_file" ] && break
    sleep 0.02
  done
  [ -s "$child_file" ]
  child_pid="$(cat "$child_file")"
  spawned_pids="$spawned_pids $child_pid"

  # shellcheck source=/dev/null
  . "$REAPER"
  autospec_kill_process_tree "$parent_pid" 0
  if ! wait "$parent_pid" 2>/dev/null; then
    :
  fi

  run kill -0 "$parent_pid"
  [ "$status" -ne 0 ]
  run bash -c 'state="$(ps -o stat= -p "$1" 2>/dev/null)"; [[ -z "$state" || "$state" == Z* ]]' _ "$child_pid"
  [ "$status" -eq 0 ]
}
