#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

ps() {
  case "$*" in
    *"-p 10") printf '10\n' ;;
    *"-p $$") printf '%s\n' "$$" ;;
  esac
}

pgrep() {
  [ "$*" = "-P 20" ] && printf '21\n'
}

kill() {
  printf 'kill %s\n' "$*" >> "$BATS_TEST_TMPDIR/signals"
}

sleep() {
  printf 'sleep %s\n' "$*" >> "$BATS_TEST_TMPDIR/signals"
}

@test "autonomous drains share the process-tree reaper" {
  for script in \
    autospec-autonomous-explore-drain.sh \
    autospec-autonomous-verify-drain.sh \
    autospec-autonomous-run-drain.sh \
    autospec-explore.sh
  do
    grep -q 'lib/autospec-process-tree.sh' "$REPO_ROOT/scripts/$script"
  done

  [ "$(rg -l '^kill_tree\\(\\)|^_explore_kill_tree\\(\\)' \
    "$REPO_ROOT/scripts/autospec-autonomous-explore-drain.sh" \
    "$REPO_ROOT/scripts/autospec-autonomous-verify-drain.sh" \
    "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh" \
    "$REPO_ROOT/scripts/autospec-explore.sh" | wc -l)" -eq 0 ]

  grep -q 'runtime_libs=.*autospec-process-tree.sh' "$REPO_ROOT/install.sh"
}

@test "shared reaper preserves separate-group and recursive modes" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/scripts/lib/autospec-process-tree.sh"

  autospec_kill_tree 10 separate
  autospec_kill_tree 20 none 1
  autospec_kill_tree 30 separate

  grep -q '^kill -TERM -- -10$' "$BATS_TEST_TMPDIR/signals"
  grep -q '^kill -KILL -- -10$' "$BATS_TEST_TMPDIR/signals"
  grep -q '^kill -TERM 21$' "$BATS_TEST_TMPDIR/signals"
  grep -q '^kill -KILL 20$' "$BATS_TEST_TMPDIR/signals"
  grep -q '^kill -TERM 30$' "$BATS_TEST_TMPDIR/signals"
  [ "$(grep -c '^kill -KILL 30$' "$BATS_TEST_TMPDIR/signals" || true)" -eq 0 ]
  [ "$(grep -c '^sleep 1$' "$BATS_TEST_TMPDIR/signals")" -eq 2 ]
}
