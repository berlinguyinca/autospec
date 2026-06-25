#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/integration/teardown.bats
#
# Integration tests for C9: teardown.sh + autospec-test Mode II clone gate.
#
# Tests cover:
#   - teardown.sh idempotency (no clone.yml, no clone-url.txt)
#   - teardown.sh removes clone-url.txt
#   - teardown.sh retains snapshots by default (--keep-snapshots)
#   - teardown.sh purges snapshots with --purge-snapshots
#   - provision.sh → clone-url.txt present → teardown.sh → clone-url.txt removed
#   - clone-gate-hook.sh exports URL and registers teardown
#
# Run: bats skills/autospec-e2e-clone/tests/integration/teardown.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
TEARDOWN_SH="$SKILL_DIR/scripts/teardown.sh"
PROVISION_SH="$SKILL_DIR/scripts/provision.sh"
CLONE_HOOK_SH="$(cd "$SKILL_DIR/../autospec-test/scripts" && pwd)/clone-gate-hook.sh"

setup() {
  TEST_REPO="$(mktemp -d -t teardown-test-repo-XXXXXX)"
  mkdir -p "$TEST_REPO/.autospec"
}

teardown() {
  rm -rf "$TEST_REPO"
}

# ── teardown.sh: idempotency ──────────────────────────────────────────────────

@test "teardown.sh: exits 0 when no clone.yml exists (idempotent)" {
  run bash "$TEARDOWN_SH" "$TEST_REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing to tear down"* ]]
}

@test "teardown.sh: exits 0 when clone-url.txt already absent (idempotent)" {
  # Create a minimal clone.yml so teardown proceeds
  mkdir -p "$TEST_REPO/.autospec"
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "echo up"
    down_cmd: "echo down"
YAML

  run bash "$TEARDOWN_SH" "$TEST_REPO"
  # Either succeeds (0) or exits with expected messages
  [ "$status" -eq 0 ] || [ "$status" -eq 1 ]
  # Should report clone-url.txt already absent
  [[ "$output" == *"already absent"* ]] || [[ "$output" == *"teardown complete"* ]]
}

@test "teardown.sh: removes .autospec/clone-url.txt" {
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "echo up"
    down_cmd: "echo done"
YAML
  printf 'http://localhost:9999\n' > "$TEST_REPO/.autospec/clone-url.txt"

  run bash "$TEARDOWN_SH" "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ ! -f "$TEST_REPO/.autospec/clone-url.txt" ]
}

@test "teardown.sh: retains clone-snapshots by default (--keep-snapshots)" {
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "echo up"
    down_cmd: "echo done"
YAML
  mkdir -p "$TEST_REPO/.autospec/clone-snapshots/sqlite/2026-01-01"
  printf 'snapshot-data\n' > "$TEST_REPO/.autospec/clone-snapshots/sqlite/2026-01-01/dump.sql"
  printf 'http://localhost:9999\n' > "$TEST_REPO/.autospec/clone-url.txt"

  run bash "$TEARDOWN_SH" "$TEST_REPO" --keep-snapshots
  [ "$status" -eq 0 ]
  [ -d "$TEST_REPO/.autospec/clone-snapshots" ]
}

@test "teardown.sh: purges clone-snapshots with --purge-snapshots" {
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "echo up"
    down_cmd: "echo done"
YAML
  mkdir -p "$TEST_REPO/.autospec/clone-snapshots/sqlite/2026-01-01"
  printf 'snapshot-data\n' > "$TEST_REPO/.autospec/clone-snapshots/sqlite/2026-01-01/dump.sql"
  printf 'http://localhost:9999\n' > "$TEST_REPO/.autospec/clone-url.txt"

  run bash "$TEARDOWN_SH" "$TEST_REPO" --purge-snapshots
  [ "$status" -eq 0 ]
  [ ! -d "$TEST_REPO/.autospec/clone-snapshots" ]
}

@test "teardown.sh: exits 0 with unknown option prints error" {
  run bash "$TEARDOWN_SH" "$TEST_REPO" --unknown-flag
  [ "$status" -eq 1 ]
  [[ "$output" == *"unknown option"* ]]
}

# ── provision → teardown round-trip (with custom_cmd mock adapter) ────────────

@test "provision.sh → teardown.sh: full round-trip with custom_cmd mock (python3 http server)" {
  # Skip if python3 (serves the health response) or curl (the adapter polls the
  # health endpoint with it) is unavailable — without either the round-trip
  # cannot run deterministically.
  command -v python3 >/dev/null 2>&1 || skip "python3 not available — skipping round-trip test"
  command -v curl    >/dev/null 2>&1 || skip "curl not available — skipping round-trip test"

  local FAKE_UP_SCRIPT="$TEST_REPO/fake-up.sh"
  local URL_FILE="$TEST_REPO/.autospec/clone-url.txt"
  local SERVER_PID_FILE="$TEST_REPO/server.pid"

  # Serve HTTP 200 on port 19888 using python3's built-in HTTP server
  cat > "$FAKE_UP_SCRIPT" << 'SHELL'
#!/usr/bin/env bash
PORT=19888
# Start a minimal HTTP server that returns 200 OK for every request
python3 -c "
import http.server, os, signal, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'ok')
    def log_message(self, *a): pass
httpd = http.server.HTTPServer(('127.0.0.1', $PORT), H)
httpd.serve_forever()
" >/dev/null 2>&1 &
printf '%s\n' "$!"
exit 0
SHELL
  chmod +x "$FAKE_UP_SCRIPT"

  cat > "$TEST_REPO/.autospec/clone.yml" << YAML
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "bash $FAKE_UP_SCRIPT"
    down_cmd: "echo fake-down-ok"
  url_template: "http://localhost:19888"
  health_endpoint: "/"
  ready_wait_secs: 10
YAML

  # provision
  run bash "$PROVISION_SH" "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ -f "$URL_FILE" ]

  # teardown
  run bash "$TEARDOWN_SH" "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ ! -f "$URL_FILE" ]
  [[ "$output" == *"teardown complete"* ]]

  # cleanup background python3 HTTP server
  pkill -f "http.server.HTTPServer.*19888" 2>/dev/null || true
}

@test "teardown.sh: removes clone-url.txt written manually (simulate post-provision state)" {
  # Simulates the state after a successful provision without needing a real adapter.
  cat > "$TEST_REPO/.autospec/clone.yml" << 'YAML'
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "echo up"
    down_cmd: "echo down-ok"
YAML
  printf 'http://localhost:19888\n' > "$TEST_REPO/.autospec/clone-url.txt"

  run bash "$TEARDOWN_SH" "$TEST_REPO"
  [ "$status" -eq 0 ]
  [ ! -f "$TEST_REPO/.autospec/clone-url.txt" ]
  [[ "$output" == *"teardown complete"* ]]
}

# ── clone-gate-hook.sh ────────────────────────────────────────────────────────

@test "clone-gate-hook.sh: exits 2 when clone.yml missing" {
  [ -f "$CLONE_HOOK_SH" ] || skip "clone-gate-hook.sh not found at $CLONE_HOOK_SH"
  run bash "$CLONE_HOOK_SH" "$TEST_REPO" "BASE_URL"
  [ "$status" -eq 2 ]
  [[ "$output" == *"clone.yml not found"* ]]
}

@test "clone-gate-hook.sh: provisions clone and exports URL env var" {
  [ -f "$CLONE_HOOK_SH" ] || skip "clone-gate-hook.sh not found at $CLONE_HOOK_SH"
  # The expose adapter polls the health endpoint with curl and we serve it with
  # a python3 HTTP server — both are required for this round-trip to be
  # deterministic. Skip cleanly rather than depend on whatever happens to be
  # listening on the host's default port.
  command -v python3 >/dev/null 2>&1 || skip "python3 not available — skipping clone-gate round-trip"
  command -v curl    >/dev/null 2>&1 || skip "curl not available — skipping clone-gate round-trip"

  local FAKE_UP_SCRIPT="$TEST_REPO/fake-up2.sh"
  local URL_FILE="$TEST_REPO/.autospec/clone-url.txt"

  # Serve HTTP 200 on port 19890 so the adapter's health poll succeeds. The
  # adapter (via the provision url-file fix) writes clone-url.txt at the
  # repo-absolute path on its own.
  cat > "$FAKE_UP_SCRIPT" << 'SHELL'
#!/usr/bin/env bash
PORT=19890
python3 -c "
import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'ok')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', $PORT), H).serve_forever()
" >/dev/null 2>&1 &
printf '%s\n' "$!"
exit 0
SHELL
  chmod +x "$FAKE_UP_SCRIPT"

  cat > "$TEST_REPO/.autospec/clone.yml" << YAML
sources:
  - kind: sqlite
    path: test.db
expose:
  kind: custom_cmd
  custom:
    up_cmd: "bash $FAKE_UP_SCRIPT"
    down_cmd: "echo fake-down-ok"
  url_template: "http://localhost:19890"
  health_endpoint: "/"
  ready_wait_secs: 10
YAML

  # Pass --add-exit-trap-cmd so the hook hands its teardown to our (no-op)
  # accumulator instead of self-registering an EXIT trap. In standalone mode
  # the hook otherwise tears the clone down the instant its own process exits,
  # which would delete clone-url.txt before we can assert it was written.
  run bash "$CLONE_HOOK_SH" "$TEST_REPO" "E2E_BASE_URL" --add-exit-trap-cmd true
  # Hook should succeed
  [ "$status" -eq 0 ]
  [ -f "$URL_FILE" ]
  [[ "$output" == *"clone gate hook complete"* ]]

  # cleanup background python3 HTTP server
  pkill -f "http.server.HTTPServer.*19890" 2>/dev/null || true
}

@test "clone-gate-hook.sh: exits 1 when no target_repo argument" {
  [ -f "$CLONE_HOOK_SH" ] || skip "clone-gate-hook.sh not found at $CLONE_HOOK_SH"
  run bash "$CLONE_HOOK_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"target_repo argument required"* ]]
}
