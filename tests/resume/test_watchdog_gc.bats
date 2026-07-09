#!/usr/bin/env bats
# tests/resume/test_watchdog_gc.bats — watchdog orphaned-worktree GC pass.
#
# Covers (docs/specs/2026-06-03-crash-resume-design.md §Error handling "GC
# safety", §Decomposition Child 3): a /tmp/wt-* worktree is pruned via
# `git worktree remove --force` ONLY when ALL THREE hold:
#   (1) no un-pushed commits   — `git -C <wt> log --not --remotes` is empty
#   (2) issue closed/unlabeled — gh shows it not OPEN+labeled auto-implement
#   (3) no live heartbeat      — no fresh heartbeat references the branch
#
# Data-integrity contract (load-bearing): the GC must NEVER remove a worktree
# with un-pushed commits, and NEVER remove an in-progress-this-host worktree
# with a fresh heartbeat. Pruning is `git worktree remove --force` only — never
# `rm -rf`.
#
# All gh / git / hostname access is via PATH-shadow mocks driven by env vars,
# mirroring tests/resume/test_resume_scan.bats. The real watchdog script runs
# in GC-only mode (AUTOSPEC_WATCHDOG_GC_ONLY=1).

ROOT="${BATS_TEST_DIRNAME}/../.."
WATCHDOG="$ROOT/scripts/autospec-watchdog.sh"

setup() {
    TEST_TMP="$(mktemp -d)"

    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="o/n"
    export AUTOSPEC_WATCHDOG_GC_ONLY="1"
    export AUTOSPEC_WATCHDOG_GC_DIR="$TEST_TMP/wt"   # scan root for /tmp/wt-* analogues
    export AUTOSPEC_HOST="host-A"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"

    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/o_n"
    mkdir -p "$AUTOSPEC_WATCHDOG_GC_DIR"

    # Records of git worktree remove invocations.
    export REMOVE_LOG="$TEST_TMP/removed.log"
    : > "$REMOVE_LOG"

    # gh mock state.
    export GH_ISSUE_STATE="CLOSED"   # state of the issue
    export GH_ISSUE_LABELS=""        # comma-joined labels

    # git mock state.
    export GIT_UNPUSHED=""           # non-empty => un-pushed commits present
    export GIT_BRANCH="feat/autospec-issue-700"

    export MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"

    write_hostname_mock
    write_gh_mock
    write_git_mock
}

teardown() { rm -rf "$TEST_TMP"; }

now_ts() { date -u +%s; }

write_hostname_mock() {
    cat > "$MOCK_DIR/hostname" <<EOF
#!/usr/bin/env bash
echo "${AUTOSPEC_HOST}"
EOF
    chmod +x "$MOCK_DIR/hostname"
}

write_gh_mock() {
    cat > "$MOCK_DIR/gh" <<'EOF'
#!/usr/bin/env bash
# PATH-shadow gh mock. Only supports `gh issue view <n> --json state,labels`.
case "$1 $2" in
  "issue view")
    printf '%s %s\n' "$GH_ISSUE_STATE" "$GH_ISSUE_LABELS"
    ;;
  "repo view")
    echo "o/n"
    ;;
  *)
    exit 0
    ;;
esac
EOF
    chmod +x "$MOCK_DIR/gh"
}

# git mock: shadows the real git so we can observe `worktree remove` and drive
# log/rev-parse outputs. Anything we do not special-case falls through to the
# real git so the watchdog's own logic still works.
write_git_mock() {
    REAL_GIT="$(command -v git)"
    export REAL_GIT
    cat > "$MOCK_DIR/git" <<EOF
#!/usr/bin/env bash
REAL_GIT="$REAL_GIT"
REMOVE_LOG="$REMOVE_LOG"
EOF
    cat >> "$MOCK_DIR/git" <<'EOF'
# Find a -C <dir> target if present (we ignore the dir; mock is global state).
args=("$@")

# git worktree ...
if [ "${args[0]}" = "worktree" ] || { [ "${args[0]}" = "-C" ] && [ "${args[2]}" = "worktree" ]; }; then
  # locate subcommand after 'worktree'
  sub=""
  for ((i=0;i<${#args[@]};i++)); do
    if [ "${args[$i]}" = "worktree" ]; then sub="${args[$((i+1))]}"; fi
  done
  case "$sub" in
    remove)
      # last arg is the path
      path="${args[$((${#args[@]}-1))]}"
      printf '%s\n' "$path" >> "$REMOVE_LOG"
      exit 0
      ;;
    *)
      exit 0
      ;;
  esac
fi

# git -C <dir> rev-parse --abbrev-ref HEAD  -> branch
# git -C <dir> rev-parse --is-inside-work-tree -> true
if printf '%s ' "${args[@]}" | grep -q 'rev-parse'; then
  if printf '%s ' "${args[@]}" | grep -q 'is-inside-work-tree'; then
    echo "true"; exit 0
  fi
  if printf '%s ' "${args[@]}" | grep -q 'abbrev-ref'; then
    echo "$GIT_BRANCH"; exit 0
  fi
  exit 0
fi

# git -C <dir> log --not --remotes  -> emit un-pushed commits when GIT_UNPUSHED set
if printf '%s ' "${args[@]}" | grep -q 'log'; then
  if [ -n "$GIT_UNPUSHED" ]; then
    echo "deadbeef un-pushed work"
  fi
  exit 0
fi

# fallthrough to real git (never reached for our cases)
exec "$REAL_GIT" "$@"
EOF
    chmod +x "$MOCK_DIR/git"
}

# Create a fake /tmp/wt-* worktree directory under the GC scan root.
make_worktree() {
    local name="$1"
    mkdir -p "$AUTOSPEC_WATCHDOG_GC_DIR/$name"
    printf '%s' "$AUTOSPEC_WATCHDOG_GC_DIR/$name"
}

# Write a heartbeat referencing a branch with a given ts and host.
write_heartbeat() {
    local issue="$1" branch="$2" ts="$3" host="$4"
    printf '{"issue":"%s","branch":"%s","step":"tests_started","ts":%s,"pr":"","repo":"o/n","host":"%s"}\n' \
        "$issue" "$branch" "$ts" "$host" \
        > "$AUTOSPEC_WATCHDOG_DIR/o_n/${issue}.json"
}

run_gc() { run bash "$WATCHDOG"; }

was_removed() { grep -qF "$1" "$REMOVE_LOG"; }

@test "gc_unpushed_not_pruned: a worktree with un-pushed commits is NOT removed" {
    wt="$(make_worktree wt-700)"
    export GIT_BRANCH="feat/autospec-issue-700"
    export GIT_UNPUSHED="yes"        # un-pushed commits present
    export GH_ISSUE_STATE="CLOSED"   # issue closed
    # no heartbeat

    run_gc
    [ "$status" -eq 0 ]
    ! was_removed "$wt"
}

@test "gc_in_progress_not_pruned: an in-progress-this-host worktree with a fresh heartbeat is NOT removed" {
    wt="$(make_worktree wt-701)"
    export GIT_BRANCH="feat/autospec-issue-701"
    export GIT_UNPUSHED=""           # clean
    export GH_ISSUE_STATE="OPEN"
    export GH_ISSUE_LABELS="auto-implement,in-progress-by-bot"
    write_heartbeat 701 "feat/autospec-issue-701" "$(now_ts)" "host-A"  # fresh, this host

    run_gc
    [ "$status" -eq 0 ]
    ! was_removed "$wt"
}

@test "gc_clean_closed_pruned: clean + closed/unlabeled issue + no live heartbeat IS removed via git worktree remove --force" {
    wt="$(make_worktree wt-702)"
    export GIT_BRANCH="feat/autospec-issue-702"
    export GIT_UNPUSHED=""           # clean, all pushed
    export GH_ISSUE_STATE="CLOSED"   # closed
    export GH_ISSUE_LABELS=""        # unlabeled
    # no heartbeat referencing the branch

    run_gc
    [ "$status" -eq 0 ]
    was_removed "$wt"
}
