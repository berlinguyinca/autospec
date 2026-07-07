#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CLI="$REPO_ROOT/scripts/autospec-autonomous.sh"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME"
  export CONDUCTOR_REPO="berlinguyinca/autospec"
  export AUTOSPEC_REPO_DIR="$REPO_ROOT"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "operator cli: status emits machine-readable stopped state" {
  run bash "$CLI" status --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"running":false'* ]]
  [[ "$output" == *'"pid":""'* ]]
  [[ "$output" == *'autonomous/berlinguyinca_autospec/state.json'* ]]
}

@test "operator cli: logs reads recorded conductor log path" {
  mkdir -p "$HOME/.autospec/autonomous-operator" "$TEST_TMP/logs"
  printf '%s\n' "$TEST_TMP/logs/conductor.log" > "$HOME/.autospec/autonomous-operator/conductor.logpath"
  printf 'first\nsecond\n' > "$TEST_TMP/logs/conductor.log"

  run bash "$CLI" logs --lines 1

  [ "$status" -eq 0 ]
  [ "$output" = "second" ]
}

@test "operator cli: stop delegates to autospec-stop helper" {
  mkdir -p "$TEST_TMP/scripts"
  cat > "$TEST_TMP/scripts/autospec-stop.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" > "$HOME/stop.args"
EOF
  chmod +x "$TEST_TMP/scripts/autospec-stop.sh"

  AUTOSPEC_SCRIPTS_DIR="$TEST_TMP/scripts" run bash "$CLI" stop --immediate

  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/stop.args")" = "--immediate" ]
}
