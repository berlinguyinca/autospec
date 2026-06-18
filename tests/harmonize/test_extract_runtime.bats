#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  EXTRACTOR="$REPO_ROOT/skills/autospec-harmonize/scripts/extract-runtime.mjs"
  FIXTURE_DIR="$REPO_ROOT/skills/autospec-harmonize/test-targets/messy-app"
  PROFILE_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-token-profile.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-extract-runtime-XXXXXX)"
}

teardown() {
  # Kill any http server started during tests
  if [ -n "${HTTP_SERVER_PID:-}" ]; then
    kill "$HTTP_SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TEST_TMPDIR"
}

# Helper: check if Playwright is importable
_playwright_available() {
  node -e "require.resolve('playwright')" 2>/dev/null
}

# Helper: find a free port
_free_port() {
  python3 -c "import socket; s=socket.socket(); s.bind(('',0)); print(s.getsockname()[1]); s.close()"
}

@test "missing Playwright exits 3 and prints harmonize_runtime_unavailable" {
  command -v node >/dev/null 2>&1 || skip "node not available"

  # Use a NODE_PATH that hides playwright, or rely on it being absent
  run env NODE_PATH=/nonexistent node "$EXTRACTOR" --url "http://127.0.0.1:1" 2>&1
  # Accept either: Playwright truly absent (exit 3) OR unreachable URL (exit 3)
  # Both paths must emit harmonize_runtime_unavailable
  [ "$status" -eq 3 ]
  [[ "$output" == *"harmonize_runtime_unavailable"* ]]
}

@test "unreachable URL exits 3 and prints harmonize_runtime_unavailable" {
  command -v node >/dev/null 2>&1 || skip "node not available"

  # Port 1 is always refused; if Playwright is absent we still get exit 3 + the line
  run node "$EXTRACTOR" --url "http://127.0.0.1:1" 2>&1
  [ "$status" -eq 3 ]
  [[ "$output" == *"harmonize_runtime_unavailable"* ]]
}

@test "missing --url exits non-zero" {
  command -v node >/dev/null 2>&1 || skip "node not available"

  run node "$EXTRACTOR" 2>&1
  [ "$status" -ne 0 ]
}

@test "positive path: source == runtime and schema-valid (skips if Playwright absent)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  _playwright_available || skip "playwright not installed"

  # Serve the fixture directory on a free port
  PORT="$(_free_port)"
  python3 -m http.server "$PORT" --directory "$FIXTURE_DIR" >/dev/null 2>&1 &
  HTTP_SERVER_PID=$!
  # Give the server a moment to start
  sleep 0.5

  run node "$EXTRACTOR" --url "http://127.0.0.1:${PORT}"
  kill "$HTTP_SERVER_PID" 2>/dev/null || true
  HTTP_SERVER_PID=""

  [ "$status" -eq 0 ]

  command -v jq >/dev/null 2>&1 || skip "jq not available"
  SOURCE="$(echo "$output" | jq -r '.source')"
  [ "$SOURCE" = "runtime" ]

  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available"
  echo "$output" > "$TEST_TMPDIR/runtime-profile.json"
  run ajv validate -s "$PROFILE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/runtime-profile.json"
  [ "$status" -eq 0 ]
}

@test "positive path: palette is non-empty array (skips if Playwright absent)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _playwright_available || skip "playwright not installed"

  PORT="$(_free_port)"
  python3 -m http.server "$PORT" --directory "$FIXTURE_DIR" >/dev/null 2>&1 &
  HTTP_SERVER_PID=$!
  sleep 0.5

  node "$EXTRACTOR" --url "http://127.0.0.1:${PORT}" > "$TEST_TMPDIR/profile.json"
  kill "$HTTP_SERVER_PID" 2>/dev/null || true
  HTTP_SERVER_PID=""

  run jq '.palette | length' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}
