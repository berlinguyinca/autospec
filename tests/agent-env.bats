#!/usr/bin/env bats
# tests/agent-env.bats — isolated runtime broker coverage

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/agent-env.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  export AGENT_ENV_STATE_ROOT="$TEST_TMP/state"
}

teardown() {
  rm -rf "$TEST_TMP"
}

write_manifest() {
  repo="$1"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/runtime.yml" <<'YAML'
version: 1
name: sample-app
default_mode: e2e-local-db
modes:
  e2e-local-db:
    env:
      E2E_USE_HARNESS: "1"
      SAMPLE_STATIC_VALUE: sample
    command: sh -c 'printf "%s\n%s\n%s\n" "$AGENT_ENV_ID" "$AUTOSPEC_PUBLIC_URL" "$AGENT_FRONTEND_PORT" > seen.txt'
    down: sh -c 'printf down > down.txt'
  ro-remote:
    command: sh -c 'printf "%s" "$AGENT_ENV_MODE" > ro.txt'
ports:
  frontend:
    env: AGENT_FRONTEND_PORT
    default: dynamic
  backend:
    env: AGENT_BACKEND_PORT
    default: dynamic
public_url_env:
  - AUTOSPEC_PUBLIC_URL
  - AGENT_PUBLIC_URL
YAML
}

@test "up fails loudly when no runtime manifest exists" {
  repo="$TEST_TMP/no-manifest"
  mkdir -p "$repo"

  run bash "$BIN" up --repo "$repo"

  [ "$status" -eq 2 ]
  echo "$output" | grep -q "no runtime manifest"
}

@test "up selects the default mode and writes exported runtime env" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo"
  write_manifest "$repo"

  run bash "$BIN" up --repo "$repo"

  [ "$status" -eq 0 ]
  [ -f "$repo/seen.txt" ]
  grep -q '^AGENT_ENV_ID=' <<< "$output"
  grep -q '^AUTOSPEC_PUBLIC_URL=http://127.0.0.1:' <<< "$output"
  grep -q '^AGENT_FRONTEND_PORT=' <<< "$output"
  grep -q '^AGENT_BACKEND_PORT=' <<< "$output"
  grep -q '^E2E_USE_HARNESS=1' <<< "$output"
  grep -q '^COMPOSE_PROJECT_NAME=agent_sample_app_' <<< "$output"
  grep -q '^sample-app-' "$repo/seen.txt"
  grep -Eq '^http://127\.0\.0\.1:[0-9]+$' "$repo/seen.txt"
  grep -q 'export E2E_USE_HARNESS=' "$AGENT_ENV_STATE_ROOT"/sample-app-*/env
  grep -q 'export SAMPLE_STATIC_VALUE=' "$AGENT_ENV_STATE_ROOT"/sample-app-*/env
}

@test "up honors an explicit mode" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo"
  write_manifest "$repo"

  run bash "$BIN" up --repo "$repo" --mode ro-remote

  [ "$status" -eq 0 ]
  [ "$(cat "$repo/ro.txt")" = "ro-remote" ]
}

@test "up propagates a failing mode command exit status" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/runtime.yml" <<'YAML'
version: 1
name: failing-app
default_mode: failing
modes:
  failing:
    command: sh -c 'exit 42'
ports:
  frontend:
    env: AGENT_FRONTEND_PORT
    default: dynamic
YAML

  run bash "$BIN" up --repo "$repo"

  [ "$status" -eq 42 ]
}

@test "exec reuses existing environment for commands" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo"
  write_manifest "$repo"
  bash "$BIN" up --repo "$repo" >/dev/null

  run bash "$BIN" exec --repo "$repo" -- sh -c 'printf "%s %s" "$AGENT_ENV_ID" "$AUTOSPEC_PUBLIC_URL"'

  [ "$status" -eq 0 ]
  echo "$output" | grep -Eq '^sample-app-[A-Za-z0-9_.-]+ http://127\.0\.0\.1:[0-9]+$'
}

@test "session provisions env for a command and tears it down after exit" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo"
  write_manifest "$repo"

  run bash "$BIN" session --repo "$repo" -- sh -c 'printf "%s %s" "$AGENT_ENV_ID" "$AUTOSPEC_PUBLIC_URL" > session.txt'

  [ "$status" -eq 0 ]
  grep -Eq '^sample-app-[A-Za-z0-9_.-]+ http://127\.0\.0\.1:[0-9]+$' "$repo/session.txt"
  [ "$(cat "$repo/down.txt")" = "down" ]
  [ ! -d "$AGENT_ENV_STATE_ROOT"/sample-app-* ]
}

@test "session passes through when no runtime manifest exists" {
  repo="$TEST_TMP/no-manifest"
  mkdir -p "$repo"

  run bash "$BIN" session --repo "$repo" -- sh -c 'printf passthrough > passthrough.txt'

  [ "$status" -eq 0 ]
  [ "$(cat "$repo/passthrough.txt")" = "passthrough" ]
  ! echo "$output" | grep -q "no runtime manifest"
}

@test "down runs the selected mode teardown command" {
  repo="$TEST_TMP/repo"
  mkdir -p "$repo"
  write_manifest "$repo"
  bash "$BIN" up --repo "$repo" >/dev/null

  run bash "$BIN" down --repo "$repo"

  [ "$status" -eq 0 ]
  [ "$(cat "$repo/down.txt")" = "down" ]
}
