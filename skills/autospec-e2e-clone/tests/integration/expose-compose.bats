#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/integration/expose-compose.bats
#
# Integration tests for the C7 docker_compose expose adapter.
# Requires Docker to be running.
#
# Run: bats skills/autospec-e2e-clone/tests/integration/expose-compose.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
COMPOSE_SH="$SKILL_DIR/scripts/expose/compose.sh"
DISPATCH_SH="$SKILL_DIR/scripts/expose/dispatch.sh"
FIX_COMPOSE="$SKILL_DIR/tests/integration/fixtures/compose/docker-compose.clone.yml"

# ── skip guard ────────────────────────────────────────────────────────────────

setup_file() {
  command -v docker >/dev/null 2>&1 || skip "docker not found — skipping integration tests"
  docker compose version >/dev/null 2>&1 || skip "docker compose plugin not found"
  docker info >/dev/null 2>&1 || skip "docker daemon not running"
}

setup() {
  TEST_REPO="$(mktemp -d -t expose-compose-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
  URL_FILE="$TEST_REPO/.autospec/clone-url.txt"
}

teardown() {
  # Always bring down the compose stack (ignore errors)
  docker compose -f "$FIX_COMPOSE" down -v 2>/dev/null || true
  rm -rf "$TEST_REPO"
}

# ── compose.sh: basic invocation ─────────────────────────────────────────────

@test "compose.sh: exits 0 on missing compose-file argument" {
  run bash "$COMPOSE_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"action required"* ]]
}

@test "compose.sh: exits 1 when compose file not found" {
  run bash "$COMPOSE_SH" up /nonexistent/docker-compose.clone.yml \
      --url "http://localhost:18080" --health "/" --wait 5 \
      --url-file "$URL_FILE"
  [ "$status" -eq 1 ]
  [[ "$output" == *"not found"* ]]
}

@test "compose.sh: exits 2 when health check times out" {
  # Use a compose file that runs but has no /health endpoint that returns 200
  local BAD_COMPOSE
  BAD_COMPOSE="$(mktemp -t bad-compose-XXXXXX.yml)"
  cat > "$BAD_COMPOSE" << 'COMPOSE'
services:
  web:
    image: nginx:alpine
    ports:
      - "19999:80"
COMPOSE

  run bash "$COMPOSE_SH" up "$BAD_COMPOSE" \
      --url "http://localhost:19999" --health "/nonexistent-endpoint" --wait 5 \
      --url-file "$URL_FILE"
  # Either 2 (timeout) or 1 (compose failure acceptable)
  [ "$status" -ne 0 ]

  docker compose -f "$BAD_COMPOSE" down -v 2>/dev/null || true
  rm -f "$BAD_COMPOSE"
}

@test "compose.sh up: starts nginx, writes clone-url.txt, returns 200" {
  run bash "$COMPOSE_SH" up "$FIX_COMPOSE" \
      --url "http://localhost:18080" --health "/" --wait 30 \
      --url-file "$URL_FILE"
  [ "$status" -eq 0 ]
  [ -f "$URL_FILE" ]
  local written_url
  written_url="$(cat "$URL_FILE")"
  [ "$written_url" = "http://localhost:18080" ]
}

@test "compose.sh up: health endpoint reachable after up" {
  bash "$COMPOSE_SH" up "$FIX_COMPOSE" \
      --url "http://localhost:18080" --health "/" --wait 30 \
      --url-file "$URL_FILE"

  local http_code
  http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://localhost:18080/")
  [ "$http_code" = "200" ]
}

@test "compose.sh down: stops stack cleanly" {
  bash "$COMPOSE_SH" up "$FIX_COMPOSE" \
      --url "http://localhost:18080" --health "/" --wait 30 \
      --url-file "$URL_FILE"

  run bash "$COMPOSE_SH" down "$FIX_COMPOSE"
  [ "$status" -eq 0 ]
  [[ "$output" == *"stopped"* ]]
}

# ── dispatch.sh ───────────────────────────────────────────────────────────────

@test "dispatch.sh: exits 1 with no action" {
  run bash "$DISPATCH_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"action required"* ]]
}

@test "dispatch.sh: exits 2 for unknown expose.kind" {
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-contract-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"unknown_kind","compose_file":"x.yml"}}\n' > "$CONTRACT"

  run bash "$DISPATCH_SH" up --contract "$CONTRACT"
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown expose.kind"* ]]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: exits 2 for deferred k8s kind" {
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-contract-k8s-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"k8s_ephemeral_namespace"}}\n' > "$CONTRACT"

  run bash "$DISPATCH_SH" up --contract "$CONTRACT"
  [ "$status" -eq 2 ]
  [[ "$output" == *"not yet implemented"* ]]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: routes docker_compose kind to compose.sh (up + down)" {
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-contract-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"docker_compose","compose_file":"%s","url_template":"http://localhost:18080","health_endpoint":"/","ready_wait_secs":30}}\n' \
    "$FIX_COMPOSE" > "$CONTRACT"

  run bash "$DISPATCH_SH" up --contract "$CONTRACT" --url-file "$URL_FILE"
  [ "$status" -eq 0 ]
  [ -f "$URL_FILE" ]

  run bash "$DISPATCH_SH" down --contract "$CONTRACT"
  [ "$status" -eq 0 ]

  rm -f "$CONTRACT"
}
