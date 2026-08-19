#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SOURCE_SCRIPT="${AUTOSPEC_AUTONOMOUS_SCRIPT:-$REPO_ROOT/scripts/autospec-autonomous.sh}"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  FIXTURE_SCRIPTS="$TEST_TMP/scripts"
  mkdir -p "$HOME" "$FIXTURE_SCRIPTS/lib"
  cp "$SOURCE_SCRIPT" "$FIXTURE_SCRIPTS/autospec-autonomous.sh"
  cp "$REPO_ROOT/scripts/lib/autospec-status-accountability.sh" "$FIXTURE_SCRIPTS/lib/"
  cat > "$FIXTURE_SCRIPTS/lib/autospec-loop.sh" <<'EOF_LOOP'
autospec_conductor_run() {
  printf 'run-foreground\n' > "$AUTOSPEC_TEST_RUN_MARKER"
  while :; do sleep 1; done
}
EOF_LOOP
  export AUTOSPEC_TEST_RUN_MARKER="$TEST_TMP/run-foreground.marker"
  export AUTOSPEC_AUTONOMOUS_OPERATOR_DIR="$TEST_TMP/operator"
  export AUTOSPEC_AUTONOMOUS_LOG_DIR="$TEST_TMP/logs"
  export AUTOSPEC_AUTONOMOUS_COMPANIONS=0
  export AUTOSPEC_REPO_DIR="$REPO_ROOT"
  export CONDUCTOR_REPO="berlinguyinca/autospec"
  unset AUTOSPEC_STOP_FLAG_FILE
}

teardown() {
  local pid_file="$AUTOSPEC_AUTONOMOUS_OPERATOR_DIR/berlinguyinca_autospec/conductor.pid"
  if [ -f "$pid_file" ]; then
    if kill "$(cat "$pid_file")" 2>/dev/null; then
      :
    fi
  fi
  rm -rf "$TEST_TMP"
}

wait_for_file() {
  local path="$1"
  local attempt=0
  while [ ! -f "$path" ] && [ "$attempt" -lt 50 ]; do
    sleep 0.02
    attempt=$((attempt + 1))
  done
  [ -f "$path" ]
}

@test "supervisor restarts run-foreground when the scoped conductor PID is absent" {
  run bash "$FIXTURE_SCRIPTS/autospec-autonomous.sh" supervise \
    --repo "$CONDUCTOR_REPO" --repo-dir "$REPO_ROOT" --interval-sec 0 --iterations 1

  [ "$status" -eq 0 ]
  wait_for_file "$AUTOSPEC_TEST_RUN_MARKER"
  [ "$(cat "$AUTOSPEC_TEST_RUN_MARKER")" = "run-foreground" ]
  [ -s "$AUTOSPEC_AUTONOMOUS_OPERATOR_DIR/berlinguyinca_autospec/conductor.pid" ]
  [[ "$output" == *"autospec-supervise: restarted stopped conductor repo=$CONDUCTOR_REPO"* ]]
}

@test "supervisor does not restart run-foreground when the stop sentinel exists" {
  mkdir -p "$AUTOSPEC_AUTONOMOUS_OPERATOR_DIR/berlinguyinca_autospec"
  touch "$AUTOSPEC_AUTONOMOUS_OPERATOR_DIR/berlinguyinca_autospec/stop.flag"

  run bash "$FIXTURE_SCRIPTS/autospec-autonomous.sh" supervise \
    --repo "$CONDUCTOR_REPO" --repo-dir "$REPO_ROOT" --interval-sec 0 --iterations 1

  [ "$status" -eq 0 ]
  [ ! -e "$AUTOSPEC_TEST_RUN_MARKER" ]
  [ ! -e "$AUTOSPEC_AUTONOMOUS_OPERATOR_DIR/berlinguyinca_autospec/conductor.pid" ]
  [[ "$output" == *"conductor=stopped"* ]]
}
