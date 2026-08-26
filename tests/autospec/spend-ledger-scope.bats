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
