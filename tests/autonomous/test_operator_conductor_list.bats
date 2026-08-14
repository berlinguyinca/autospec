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

@test "operator cli: list enumerates repo-scoped conductors with provenance and heartbeat metadata" {
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec" \
    "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules" \
    "$HOME/.autospec/autonomous/berlinguyinca_autospec"
  sleep 30 &
  live_pid="$!"
  trap 'kill "$live_pid" 2>/dev/null || true' RETURN
  printf '%s\n' "$live_pid" > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"
  printf '%s\n' "$TEST_TMP/autospec.log" > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.logpath"
  printf '999999\n' > "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.pid"
  printf '%s\n' "$TEST_TMP/go-modules.log" > "$HOME/.autospec/autonomous-operator/metabolomics-us_go-modules/conductor.logpath"
  cat > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/launch.json" <<EOF_JSON
{"argv":["start","--repo","berlinguyinca/autospec"],"started_at":"2026-07-08T12:00:00Z","tty":"/dev/ttys001","session_id":"abc123","repo":"berlinguyinca/autospec","repo_dir":"$REPO_ROOT","accountability":{"run_id":"abc123","epic_number":3135,"epic_url":"https://github.com/berlinguyinca/autospec/issues/3135"}}
EOF_JSON
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/accountability"
  printf '%s\n' '{"event_count":9,"pending_projection_count":1}' > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/accountability/accountability.json"
  cat > "$HOME/.autospec/autonomous/berlinguyinca_autospec/state.json" <<'EOF_STATE'
{"status":"parked:usage-limit","heartbeat_at":1783526400,"cycle":42}
EOF_STATE

  run bash "$CLI" list --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"conductors":['* ]]
  [[ "$output" == *'"slug":"berlinguyinca_autospec"'* ]]
  [[ "$output" == *'"repo":"berlinguyinca/autospec"'* ]]
  [[ "$output" == *'"alive":true'* ]]
  [[ "$output" == *'"last_cycle":"42"'* ]]
  [[ "$output" == *'"park_state":"parked:usage-limit"'* ]]
  [[ "$output" == *'"started_at":"2026-07-08T12:00:00Z"'* ]]
  [[ "$output" == *'"argv":["start","--repo","berlinguyinca/autospec"]'* ]]
  [[ "$output" == *'"epic_number":3135'* ]]
  [[ "$output" == *'"event_count":9'* ]]
  [[ "$output" == *'"projection_state":"degraded"'* ]]
  [[ "$output" == *'"slug":"metabolomics-us_go-modules"'* ]]
  [[ "$output" == *'"alive":false'* ]]
}

@test "operator cli: status --all --json is an alias for conductor list JSON" {
  mkdir -p "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec"
  printf '999999\n' > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"

  run bash "$CLI" status --all --json

  [ "$status" -eq 0 ]
  [[ "$output" == *'"conductors":['* ]]
  [[ "$output" == *'"slug":"berlinguyinca_autospec"'* ]]
}

@test "operator cli: detached start records launch provenance" {
  mkdir -p "$TEST_TMP/bin"
  cat > "$TEST_TMP/bin/python3" <<'PY_FAKE'
#!/usr/bin/env bash
printf '424242\n'
PY_FAKE
  chmod +x "$TEST_TMP/bin/python3"

  PATH="$TEST_TMP/bin:$PATH" run bash "$CLI" start --repo berlinguyinca/autospec --repo-dir "$REPO_ROOT" --dry-run --force

  [ "$status" -eq 0 ]
  provenance="$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/launch.json"
  [ -f "$provenance" ]
  grep -q '"repo":"berlinguyinca/autospec"' "$provenance"
  grep -q '"repo_dir":"'$REPO_ROOT'"' "$provenance"
  grep -q '"argv":\["start","--repo","berlinguyinca/autospec","--repo-dir","'$REPO_ROOT'","--dry-run","--force"\]' "$provenance"
  grep -q '"started_at":' "$provenance"
  grep -q '"session_id":' "$provenance"
}

@test "operator cli: logs falls back to newest legacy flat log when scoped logpath is missing" {
  mkdir -p "$HOME/.autospec/logs" "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec"
  printf '111111\n' > "$HOME/.autospec/autonomous-operator/berlinguyinca_autospec/conductor.pid"
  printf 'old legacy\n' > "$HOME/.autospec/logs/autospec-autonomous-20260708T100000Z.log"
  printf 'new legacy\n' > "$HOME/.autospec/logs/autospec-autonomous-20260708T110000Z.log"

  run bash "$CLI" logs --repo berlinguyinca/autospec --lines 1

  [ "$status" -eq 0 ]
  [ "$output" = "new legacy" ]
}
