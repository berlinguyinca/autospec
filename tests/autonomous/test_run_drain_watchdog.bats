#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
  mkdir -p "$HOME" "$TEST_TMP/bin"
  rm -rf /tmp/autospec-run-1838
}

teardown() {
  rm -rf "$TEST_TMP"
  rm -rf /tmp/autospec-run-1838
}

@test "run-drain: exits when omx child makes no progress past stall timeout" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$$" > "$HOME/omx.pid"
exec sleep 30
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=1 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh" > "$TEST_TMP/drain.out" 2>&1 &
  drain_pid="$!"

  for _ in 1 2 3 4 5; do
    if ! kill -0 "$drain_pid" 2>/dev/null; then
      status=0
      wait "$drain_pid" || status="$?"
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

@test "run-drain: follows declared validation log progress from a quiet omx child" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
for _ in 1 2 3; do
  printf 'validate tick\n' >> "$AUTOSPEC_AUTONOMOUS_DRAIN_LOG"
  sleep 1
done
exit 0
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=2 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  AUTOSPEC_AUTONOMOUS_DRAIN_LOG="$TEST_TMP/validate-1840-cleanenv.log" \
  run bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh"

  [ "$status" -eq 0 ]
  [ "$(grep -c 'validate tick' "$TEST_TMP/validate-1840-cleanenv.log")" -eq 3 ]
}

@test "run-drain: treats heartbeat updates as progress from a quiet omx child" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
heartbeat_dir="$HOME/.autospec/process-heartbeats/berlinguyinca_autospec"
mkdir -p "$heartbeat_dir"
for tick in 1 2 3; do
  printf '{"issue":"1842","tick":%s}\n' "$tick" > "$heartbeat_dir/1842.json"
  sleep 1
done
exit 0
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=2 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  run bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh"

  [ "$status" -eq 0 ]
  grep -q '"tick":3' "$HOME/.autospec/process-heartbeats/berlinguyinca_autospec/1842.json"
}

@test "run-drain: records closeout hang when issue wrapper has no child progress" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$$" > "$HOME/omx.pid"
exec sleep 30
EOF
  chmod +x "$TEST_TMP/bin/omx"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=1 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  AUTOSPEC_AUTONOMOUS_DRAIN_ISSUE=1838 \
  bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh" > "$TEST_TMP/drain.out" 2>&1 &
  drain_pid="$!"

  for _ in 1 2 3 4 5; do
    if ! kill -0 "$drain_pid" 2>/dev/null; then
      status=0
      wait "$drain_pid" || status="$?"
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
  grep -q "closeout hang" "$TEST_TMP/drain.out"
  grep -q "#1838" "$TEST_TMP/drain.out"
  grep -q "closeout hang" "/tmp/autospec-run-1838/closeout-hang.md"
  grep -q "#1838" "/tmp/autospec-run-1838/closeout-hang.md"
}

@test "run-drain: recovers stale wait handle by merging green in-progress PR" {
  cat > "$TEST_TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
printf 'codex_core::tools::router: error=write_stdin failed: Unknown process id 83740\n' >&2
exit 1
EOF
  chmod +x "$TEST_TMP/bin/omx"

  cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$HOME/gh.log"
case "$*" in
  *"repo view"*)
    printf 'berlinguyinca/autospec\n'
    ;;
  "pr list --repo berlinguyinca/autospec --state open --json number,headRefName,statusCheckRollup,isDraft"*)
    cat <<'JSON'
[{"number":1578,"headRefName":"feat/issue-1545-blast-radius-quarantine","isDraft":false,"statusCheckRollup":[{"name":"pytest","status":"COMPLETED","conclusion":"SUCCESS"},{"name":"GitGuardian Security Checks","status":"COMPLETED","conclusion":"SUCCESS"},{"name":"doc-drift","status":"COMPLETED","conclusion":"SKIPPED"}]}]
JSON
    ;;
  "issue view 1545 --repo berlinguyinca/autospec --json state,labels"*)
    printf '{"state":"OPEN","labels":[{"name":"in-progress-by-bot"}]}\n'
    ;;
  "pr merge 1578 --repo berlinguyinca/autospec --admin --squash --delete-branch"*)
    printf '{"state":"MERGED"}\n' > "$HOME/pr-1578.json"
    ;;
  "pr view 1578 --repo berlinguyinca/autospec --json state,mergedAt"*)
    cat "${HOME}/pr-1578.json"
    ;;
  "issue edit 1545 --repo berlinguyinca/autospec --remove-label in-progress-by-bot"*)
    ;;
  *)
    printf 'unexpected gh args: %s\n' "$*" >&2
    exit 2
    ;;
esac
EOF
  chmod +x "$TEST_TMP/bin/gh"

  PATH="$TEST_TMP/bin:$PATH" \
  AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS=10 \
  AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS=1 \
  run bash "$REPO_ROOT/scripts/autospec-autonomous-run-drain.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"stale wait handle recovery merged PR #1578 for issue #1545"* ]]
  grep -q "gh pr merge 1578" "$HOME/gh.log"
}
