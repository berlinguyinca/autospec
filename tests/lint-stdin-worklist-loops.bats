#!/usr/bin/env bats
# tests/lint-stdin-worklist-loops.bats
#
# Exercises scripts/lint-stdin-worklist-loops.sh (issue #3742) against the
# positive and negative cases: a while/for/until/select loop that reads its
# worklist from stdin (FD 0, `done < file`) while running a stdin-consuming
# command (gh / cargo / git stdin-subcommand / ssh, literal or via $(...) or
# a GH/SSH/CARGO variable alias) in the body is reported; a loop that reads
# its worklist on a dedicated FD (done 3<), a loop with no redirect, a loop
# whose git subcommand does not read stdin, a command attributed to a nested
# loop, and a command outside any loop are not.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-stdin-worklist-loops.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-stdin-worklist-loops"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"STDIN_WORKLIST_LOOP"* ]]
}

@test "pos1: an FD-0 loop with a literal gh in the body is flagged" {
  run bash "$LINT" "$FIX/pos1-gh-literal.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"STDIN_WORKLIST_LOOP:"* ]]
  [[ "$output" == *"runs 'gh'"* ]]
}

@test "pos2: an FD-0 loop with a \$() gh command substitution is flagged" {
  run bash "$LINT" "$FIX/pos2-gh-cmdsub.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"STDIN_WORKLIST_LOOP:"* ]]
  [[ "$output" == *"runs '\$(gh)'"* ]]
}

@test "pos3: an FD-0 loop with a git stdin-subcommand is flagged" {
  run bash "$LINT" "$FIX/pos3-git-apply.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"STDIN_WORKLIST_LOOP:"* ]]
  [[ "$output" == *"runs 'git'"* ]]
}

@test "neg1: a dedicated-FD (done 3<) loop is NOT flagged" {
  run bash "$LINT" "$FIX/neg1-dedicated-fd.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"STDIN_WORKLIST_LOOP:"* ]]
}

@test "neg2: an FD-0 loop whose git subcommand does not read stdin is NOT flagged" {
  run bash "$LINT" "$FIX/neg2-non-stdin-git.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"STDIN_WORKLIST_LOOP:"* ]]
}

@test "neg3: a counter loop with no redirect is NOT flagged" {
  run bash "$LINT" "$FIX/neg3-no-redirect.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"STDIN_WORKLIST_LOOP:"* ]]
}

@test "neg4: a gh attributed to a nested loop is NOT charged to the outer FD-0 loop" {
  run bash "$LINT" "$FIX/neg4-nested-loop.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"STDIN_WORKLIST_LOOP:"* ]]
}

@test "neg5: a gh outside the loop plus a clean FD-0 loop is NOT flagged" {
  run bash "$LINT" "$FIX/neg5-gh-outside-loop.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"STDIN_WORKLIST_LOOP:"* ]]
}

@test "whole-fixtures scan reports the three positive cases and exits 3" {
  run bash "$LINT" "$FIX"
  [ "$status" -eq 3 ]
  [ "$(printf '%s\n' "$output" | grep -c '^STDIN_WORKLIST_LOOP:')" -eq 3 ]
}

@test "--list emits an FD0_LOOP audit line and exits 0" {
  run bash "$LINT" --list "$FIX/pos1-gh-literal.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"FD0_LOOP:"* ]]
}

@test "default scan of scripts/ and skills/ passes on this tree" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
}
