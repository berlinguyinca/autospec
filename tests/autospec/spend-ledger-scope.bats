#!/usr/bin/env bats
# A fleet sharing one board must share one budget, not multiply it per repo.

bats_require_minimum_version 1.5.0

setup() {
  TMP="$(mktemp -d)"; export HOME="$TMP"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-spend-ledger.sh"
  mkdir -p "$TMP/a" "$TMP/b"
  (cd "$TMP/a" && git init -q . && git remote add origin https://github.com/o/a.git)
  (cd "$TMP/b" && git init -q . && git remote add origin https://github.com/o/b.git)
}
teardown() { rm -rf "$TMP"; }

# Backdate a directory's mtime so it reads as genuinely old, cross-platform
# (BSD `touch -t` on macOS, GNU `touch -d` elsewhere). Used so staleness
# tests can use a realistic, nonzero AUTOSPEC_SPEND_LOCK_STALE_SECONDS
# instead of 0 — 0 would remove the anti-TOCTOU safety margin for every
# lock a concurrent test creates, not just the deliberately-orphaned one,
# and would flakily let a worker reclaim a sibling's still-live lock.
backdate() {
  local path="$1" secs="$2" past
  # -h: change the symlink's own mtime, don't follow it (the lock is a
  # dangling symlink whose target is just a bare PID string).
  if past="$(date -v-"${secs}"S +%Y%m%d%H%M.%S 2>/dev/null)"; then
    touch -h -t "$past" "$path"
  else
    past="$(date -d "-${secs} seconds" +%Y%m%d%H%M.%S)"
    touch -h -t "$past" "$path"
  fi
}

# The outer `timeout` guarding ledger_lock_acquire's retry loop must be
# proportional to the loop's OWN configured bound, not a fixed wall-clock
# guess. A fixed guess (e.g. a flat `timeout 5`) is either too tight — it
# fires under CPU contention even though the loop is genuinely, correctly
# bounded, reporting a hang that never happened — or, if inflated to a
# large fixed constant, stops being a meaningful bound at all.
#
# ledger_lock_acquire's real worst case is:
#   AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER iterations * the script's own
#   `sleep 0.05` per iteration (scripts/autonomous-spend-ledger.sh,
#   ledger_lock_acquire — keep LOCK_WAIT_SLEEP_INTERVAL in sync with that
#   literal).
#
# We multiply that by a generous-but-finite factor to absorb scheduler
# noise under a loaded machine, plus a flat overhead for fork/exec/jq
# startup cost that doesn't scale with iteration count. The result stays
# a real bound: a genuine regression to unbounded spinning still fails
# this test (just diagnosed a bit slower under load) instead of wedging
# the suite, which a dropped or infinite timeout would risk.
LOCK_WAIT_SLEEP_INTERVAL=0.05
LOCK_WAIT_BUDGET_MULTIPLIER=20
LOCK_WAIT_BUDGET_OVERHEAD=2

lock_wait_timeout() {
  local iter="$1"
  awk -v i="$iter" -v s="$LOCK_WAIT_SLEEP_INTERVAL" \
    -v m="$LOCK_WAIT_BUDGET_MULTIPLIER" -v o="$LOCK_WAIT_BUDGET_OVERHEAD" \
    'BEGIN { printf "%.2f", (i * s * m) + o }'
}

@test "a project_board.spend_scope in .autospec/autonomous.yml is bridged into a shared ledger" {
  REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
  AUTOSPEC_BIN_PATH="$REPO_ROOT/target/debug/autospec"
  if [ ! -x "$AUTOSPEC_BIN_PATH" ]; then
    skip "target/debug/autospec not built; run cargo build -p autospec-cli first"
  fi
  export AUTOSPEC_PROJECT_BOARD_CONFIG_BIN="$AUTOSPEC_BIN_PATH"

  # A board-driven fleet ships the SAME project_board.spend_scope in every
  # member repo's .autospec/autonomous.yml — that is what "one budget by
  # configuration" means; no operator exports AUTOSPEC_SPEND_SCOPE anywhere.
  mkdir -p "$TMP/a/.autospec" "$TMP/b/.autospec"
  cat > "$TMP/a/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
  spend_scope: board-inferweave-shared
YML
  cp "$TMP/a/.autospec/autonomous.yml" "$TMP/b/.autospec/autonomous.yml"

  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null

  run bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 200'
  # And it landed at the configured scope's directory, not a per-repo slug.
  [ -f "$TMP/.autospec/autonomous-spend/board-inferweave-shared/spend.json" ]
}

@test "an explicitly-exported AUTOSPEC_SPEND_SCOPE wins over the bridged YAML value" {
  REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
  AUTOSPEC_BIN_PATH="$REPO_ROOT/target/debug/autospec"
  if [ ! -x "$AUTOSPEC_BIN_PATH" ]; then
    skip "target/debug/autospec not built; run cargo build -p autospec-cli first"
  fi
  export AUTOSPEC_PROJECT_BOARD_CONFIG_BIN="$AUTOSPEC_BIN_PATH"

  mkdir -p "$TMP/a/.autospec"
  cat > "$TMP/a/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
  spend_scope: board-scope-from-yaml
YML

  AUTOSPEC_SPEND_SCOPE=operator-override bash "$SCRIPT" add --tokens 50 --repo-dir "$TMP/a" >/dev/null
  [ -f "$TMP/.autospec/autonomous-spend/operator-override/spend.json" ]
  [ ! -e "$TMP/.autospec/autonomous-spend/board-scope-from-yaml" ]
}

@test "with no .autospec/autonomous.yml, spend_scope bridging is a no-op and per-repo ledgers still separate" {
  REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
  AUTOSPEC_BIN_PATH="$REPO_ROOT/target/debug/autospec"
  if [ ! -x "$AUTOSPEC_BIN_PATH" ]; then
    skip "target/debug/autospec not built; run cargo build -p autospec-cli first"
  fi
  export AUTOSPEC_PROJECT_BOARD_CONFIG_BIN="$AUTOSPEC_BIN_PATH"

  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 100'
}

@test "without a scope override two repos keep separate ledgers" {
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 100'
}

@test "a shared scope accumulates both repos into one ledger" {
  AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run env AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 200'
}

@test "a shared scope parks both repos once the shared cap is hit" {
  AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run env AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" check --repo-dir "$TMP/b"
  echo "$output" | grep -q '^park'

  run env AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" check --repo-dir "$TMP/a"
  echo "$output" | grep -q '^park'
}

@test "a scope containing a path separator (traversal) is rejected" {
  run env AUTOSPEC_SPEND_SCOPE='../../etc' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}

@test "a scope containing an embedded slash is rejected" {
  run env AUTOSPEC_SPEND_SCOPE='a/b' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}

@test "an empty scope override is rejected" {
  run env AUTOSPEC_SPEND_SCOPE='' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}

@test "a scope containing a newline is rejected" {
  run env AUTOSPEC_SPEND_SCOPE="$(printf 'a\nb')" bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}

@test "no traversal directory escapes the ledger root" {
  run env AUTOSPEC_SPEND_SCOPE='../../etc' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  [ ! -e "$TMP/etc" ]
  [ ! -e "$TMP/.autospec/etc" ]
}

@test "concurrent writers under a shared scope sum without lost updates" {
  N=8
  PIDS=""
  for i in $(seq 1 "$N"); do
    ( AUTOSPEC_SPEND_SCOPE=board-concurrent bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a" >/dev/null 2>&1 ) &
    PIDS="$PIDS $!"
  done
  fail=0
  for pid in $PIDS; do
    wait "$pid" || fail=1
  done
  [ "$fail" -eq 0 ]

  run env AUTOSPEC_SPEND_SCOPE=board-concurrent bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 80'
}

@test "a scope starting with a dash is rejected" {
  run env AUTOSPEC_SPEND_SCOPE='-x' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}

@test "a scope longer than the max length is rejected with a clear message" {
  long="$(printf 'a%.0s' $(seq 1 5000))"
  run env AUTOSPEC_SPEND_SCOPE="$long" bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  echo "$output" | grep -qi 'longer than'
}

# ── Lock staleness ───────────────────────────────────────────────────────────

@test "a lock orphaned by a dead process is reclaimed and the operation succeeds" {
  scope=board-dead
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  # Spawn and immediately reap a subshell so its PID is guaranteed dead.
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60

  run --separate-stderr env AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.tokens == 10'
  echo "$stderr" | grep -qi 'reclaimed a stale ledger lock'
}

@test "a lock held by a live process is NOT reclaimed even when old" {
  scope=board-live
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  sleep 5 &
  livepid=$!
  ln -s "$livepid" "$lockdir"
  backdate "$lockdir" 3600

  run env AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
    AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER=5 \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  echo "$output" | grep -qi 'timed out waiting for ledger lock'

  kill "$livepid" 2>/dev/null || true
  wait "$livepid" 2>/dev/null || true
  # The still-live-owned lock must remain untouched (not reclaimed).
  [ -L "$lockdir" ]
}

@test "two concurrent reclaimers of the same stale lock do not both proceed" {
  scope=board-race
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60

  err_a="$TMP/err_a.log"
  err_b="$TMP/err_b.log"
  ( AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
      bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a" >/dev/null 2>"$err_a" ) &
  pid_a=$!
  ( AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
      bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a" >/dev/null 2>"$err_b" ) &
  pid_b=$!

  fail=0
  wait "$pid_a" || fail=1
  wait "$pid_b" || fail=1
  [ "$fail" -eq 0 ]

  reclaims="$(cat "$err_a" "$err_b" | grep -c 'reclaimed a stale ledger lock' || true)"
  [ "$reclaims" -eq 1 ]

  run env AUTOSPEC_SPEND_SCOPE="$scope" bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 20'
}

@test "after reclaiming a stale lock, 8-way concurrency still totals exactly 80" {
  scope=board-reclaim-then-concurrent
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60

  N=8
  PIDS=""
  for i in $(seq 1 "$N"); do
    ( AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
        bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a" >/dev/null 2>&1 ) &
    PIDS="$PIDS $!"
  done
  fail=0
  for pid in $PIDS; do
    wait "$pid" || fail=1
  done
  [ "$fail" -eq 0 ]

  run env AUTOSPEC_SPEND_SCOPE="$scope" bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 80'
}

# ── Bounded failure paths (never hang, never silently spin) ─────────────────
#
# ledger_lock_acquire's retry loop must terminate — either by acquiring the
# lock or by die()ing after LOCK_MAX_WAIT_ITER — no matter what happens on
# the reclaim side. Every scenario below wraps the call in `timeout` so a
# regression to the busy-spin bug (an orphaned reclaim mutex causing
# `continue` to skip the wait/timeout accounting) fails the test suite fast
# instead of wedging it.

@test "orphaned .reclaiming mutex + a LIVE lock holder: bounded timeout, no hang" {
  scope=board-orphan-reclaiming-live
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  sleep 5 &
  livepid=$!
  ln -s "$livepid" "$lockdir"
  mkdir "${lockdir}.reclaiming"

  iter=20
  run timeout "$(lock_wait_timeout "$iter")" env AUTOSPEC_SPEND_SCOPE="$scope" \
    AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER="$iter" \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  [ "$status" -ne 124 ]  # 124 = timeout(1) killed it — must not happen
  echo "$output" | grep -qi 'timed out waiting for ledger lock'

  kill "$livepid" 2>/dev/null || true
  wait "$livepid" 2>/dev/null || true
}

@test "orphaned .reclaiming mutex + a DEAD lock holder: bounded timeout, no hang" {
  scope=board-orphan-reclaiming-dead
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60
  mkdir "${lockdir}.reclaiming"

  iter=10
  run timeout "$(lock_wait_timeout "$iter")" env AUTOSPEC_SPEND_SCOPE="$scope" \
    AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
    AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER="$iter" \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  [ "$status" -ne 124 ]
  echo "$output" | grep -qi 'timed out waiting for ledger lock'
}

@test "orphaned .reclaiming mutex with NO lock at all: acquires immediately, no hang" {
  scope=board-orphan-reclaiming-nolock
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  mkdir "${lockdir}.reclaiming"

  run timeout 5 env AUTOSPEC_SPEND_SCOPE="$scope" bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.tokens == 10'
}

@test "both lockdir and .reclaiming orphaned: bounded timeout, then recovers once the operator clears .reclaiming" {
  scope=board-both-orphaned
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60
  mkdir "${lockdir}.reclaiming"

  iter=10
  run timeout "$(lock_wait_timeout "$iter")" env AUTOSPEC_SPEND_SCOPE="$scope" \
    AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
    AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER="$iter" \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
  [ "$status" -ne 124 ]
  echo "$output" | grep -qi 'timed out waiting for ledger lock'

  # An orphaned reclaim mutex is not permanently fatal: once an operator
  # clears it (the visible, diagnosable failure above is what tells them
  # to), the ledger recovers on its own. This second run doesn't loop at
  # all (the reclaiming mutex is gone, the lock acquires on the first
  # attempt), so it isn't bound by LOCK_MAX_WAIT_ITER — a flat generous
  # budget (matching the fast-path test below) is enough headroom for
  # fork/exec cost under load without pretending it's proportional to a
  # loop it doesn't run.
  rmdir "${lockdir}.reclaiming"
  run --separate-stderr timeout 15 env AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
    bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.tokens == 10'
}

@test "reclaim mutex won but the lock vanishes before mv: aborts cleanly, still bounded" {
  scope=board-vanish-before-mv
  lockdir="$TMP/.autospec/autonomous-spend/${scope}/spend.json.lock"
  mkdir -p "$(dirname "$lockdir")"
  ( : ) & deadpid=$!
  wait "$deadpid" || true
  ln -s "$deadpid" "$lockdir"
  backdate "$lockdir" 60

  # AUTOSPEC_SPEND_LOCK_TEST_STALL is a test-only seam (no-op unless set):
  # it stalls ledger_lock_reclaim right after it wins the reclaim mutex,
  # giving this test a deterministic window to remove the lock out from
  # under it before the reclaim's own readlink/mv runs. This path also
  # doesn't loop through LOCK_MAX_WAIT_ITER (it recovers on the very next
  # attempt after the stall), so the budget is a flat generous constant
  # (same reasoning as the recovery run above) rather than derived from
  # the retry-loop formula.
  ( timeout 15 env AUTOSPEC_SPEND_SCOPE="$scope" AUTOSPEC_SPEND_LOCK_STALE_SECONDS=2 \
      AUTOSPEC_SPEND_LOCK_TEST_STALL=1 \
      bash "$SCRIPT" add --tokens 10 --repo-dir "$TMP/a" >"$TMP/vanish.out" 2>"$TMP/vanish.err"
    echo "$?" > "$TMP/vanish.exit" ) &
  runner=$!
  sleep 0.3
  rm -f "$lockdir"
  wait "$runner"

  exitcode="$(cat "$TMP/vanish.exit")"
  [ "$exitcode" -eq 0 ]
  [ "$exitcode" -ne 124 ]
  cat "$TMP/vanish.out" | jq -e '.tokens == 10'
}
