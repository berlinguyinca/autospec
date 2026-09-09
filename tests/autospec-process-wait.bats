#!/usr/bin/env bats
# tests/autospec-process-wait.bats — issue #3938: pid/sentinel wait
# helper contract, including the self-matching wrapper case.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LIB="$REPO_ROOT/scripts/lib/autospec-process-wait.sh"

@test "wait_pid: returns 0 when the pid exits" {
  sleep 0.3 &
  local pid=$!
  run bash -c ". '$LIB'; autospec_wait_pid $pid 10 'test sleep'"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  wait "$pid" 2>/dev/null || true
}

@test "wait_pid: deadline expiry reports condition and elapsed, returns 1" {
  sleep 30 &
  local pid=$!
  run bash -c ". '$LIB'; autospec_wait_pid $pid 1 'hung worker'; rc=\$?; kill $pid 2>/dev/null; wait $pid 2>/dev/null; exit \$rc"
  [ "$status" -eq 1 ]
  [[ "$output" == *"deadline 1s exceeded"* ]]
  [[ "$output" == *"still waiting for: hung worker (pid $pid)"* ]]
  [[ "$output" == *after\ [0-9]s* ]]
}

@test "wait_pid: treats a zombie (kill -0 succeeds, state Z) as gone" {
  # bash reaps its own dead background jobs, so a real zombie cannot be
  # left behind from a test body; a ps shim reports Z for the live pid
  # instead, which is exactly what an unreaped child looks like to the
  # helper.
  local dir pid
  dir="$(mktemp -d)"
  cat > "$dir/ps" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "-o" ] && [ "${2:-}" = "state=" ] && [ -n "${PS_SHIM_ZOMBIE_PID:-}" ] && [ "${4:-}" = "$PS_SHIM_ZOMBIE_PID" ]; then
    printf 'Z\n'
    exit 0
fi
exec /usr/bin/ps "$@"
EOF
  chmod +x "$dir/ps"
  sleep 5 &
  pid=$!
  run env PATH="$dir:$PATH" PS_SHIM_ZOMBIE_PID="$pid" bash -c ". '$LIB'; autospec_wait_pid $pid 5 'zombie child'"
  [ "$status" -eq 0 ]
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null || true
  rm -rf "$dir"
}

@test "wait_pid: refuses the caller's own pid" {
  run bash -c ". '$LIB'; autospec_wait_pid \$\$ 1 'self'"
  [ "$status" -eq 3 ]
  [[ "$output" == *"refusing to wait on own pid"* ]]
}

@test "wait_pid: usage error on a non-numeric pid" {
  run bash -c ". '$LIB'; autospec_wait_pid notapid 5"
  [ "$status" -eq 2 ]
  [[ "$output" == *"usage:"* ]]
}

@test "wait_sentinel: returns 0 when the sentinel appears" {
  local dir sentinel
  dir="$(mktemp -d)"
  sentinel="$dir/done"
  ( sleep 0.3; touch "$sentinel" ) &
  local spid=$!
  run bash -c ". '$LIB'; autospec_wait_sentinel '$sentinel' 10 'worker sentinel'"
  [ "$status" -eq 0 ]
  wait "$spid" 2>/dev/null || true
  rm -rf "$dir"
}

@test "wait_sentinel: deadline expiry reports and returns 1" {
  local dir sentinel
  dir="$(mktemp -d)"
  sentinel="$dir/never"
  run bash -c ". '$LIB'; autospec_wait_sentinel '$sentinel' 1 'absent sentinel'"
  [ "$status" -eq 1 ]
  [[ "$output" == *"deadline 1s exceeded"* ]]
  [[ "$output" == *"still waiting for: absent sentinel (file $sentinel)"* ]]
  [[ "$output" == *after\ [0-9]s* ]]
  rm -rf "$dir"
}

@test "stop_pid: stops work by pid via the process-tree helper" {
  command -v setsid >/dev/null 2>&1 || skip "setsid not available"
  setsid sleep 30 &
  local pid=$!
  run bash -c ". '$LIB'; autospec_stop_pid $pid; exit 0"
  [ "$status" -eq 0 ]
  local i state
  for i in $(seq 1 50); do
    state="$(ps -o state= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
    [ -z "$state" ] && break
    sleep 0.1
  done
  [ -z "$state" ]
}

@test "self-matching wrapper: pid wait completes where a -f pattern wait would hang" {
  command -v setsid >/dev/null 2>&1 || skip "setsid not available"
  local tmpdir wrapper worker wpid
  tmpdir="$(mktemp -d)"
  worker="$tmpdir/worker.sh"
  printf '#!/usr/bin/env bash\nsleep 0.5\n' > "$worker"
  wrapper="$tmpdir/wrapper.sh"
  cat > "$wrapper" <<'EOF'
#!/usr/bin/env bash
# argv contains "worker.sh" (the worker path): a -f pattern wait for that
# string would match this wrapper itself and hang (issue #3938).
. "$1"
bash "$2" &
autospec_wait_pid "$!" 10 "worker.sh"
EOF
  chmod +x "$wrapper"
  # Run the wrapper in its own session so a leftover cannot outlive the test.
  setsid bash "$wrapper" "$LIB" "$worker" &
  wpid=$!
  # Give the wrapper time to spawn the worker, then prove the trap is
  # real: a -f pattern for the worker string matches the wrapper itself.
  sleep 0.3
  pgrep -f "worker.sh" | grep -qx "$wpid"
  local rc=0
  wait "$wpid" || rc=$?
  [ "$rc" -eq 0 ]
  rm -rf "$tmpdir"
}
