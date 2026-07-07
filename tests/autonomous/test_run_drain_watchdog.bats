#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME" "$TEST_TMP/bin"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "run-drain: exits when omx child makes no progress past stall timeout" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$$" > "$HOME/omx.pid"
sleep 30
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=1 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh" > "$TEST_TMP/drain.out" 2>&1 &
  drain_pid="$!"

  for _ in 1 2 3 4 5; do
    if ! kill -0 "$drain_pid" 2>/dev/null; then
      set +e
      wait "$drain_pid"
      status="$?"
      set -e
      break
    fi
    sleep 1
  done

  if kill -0 "$drain_pid" 2>/dev/null; then
    kill "$drain_pid" 2>/dev/null || true
    wait "$drain_pid" 2>/dev/null || true
    cat "$TEST_TMP/drain.out"
    false
  fi

  [ "${status:-0}" -eq 124 ]
  grep -q "stalled after 1s with no output" "$TEST_TMP/drain.out"
  if [ -f "$HOME/omx.pid" ]; then
    ! kill -0 "$(cat "$HOME/omx.pid")" 2>/dev/null
  fi
}

@test "run-drain: returns successful omx child status when it completes" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
printf 'omx completed\n'
exit 0
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=10 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  run bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"omx completed"* ]]
}
