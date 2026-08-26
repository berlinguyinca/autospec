#!/usr/bin/env bats
# A fleet sharing one board must share one budget, not multiply it per repo.

setup() {
  TMP="$(mktemp -d)"; export HOME="$TMP"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-spend-ledger.sh"
  mkdir -p "$TMP/a" "$TMP/b"
  (cd "$TMP/a" && git init -q . && git remote add origin https://github.com/o/a.git)
  (cd "$TMP/b" && git init -q . && git remote add origin https://github.com/o/b.git)
}
teardown() { rm -rf "$TMP"; }

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
