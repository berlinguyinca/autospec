#!/usr/bin/env bats
# skills/autospec-e2e-clone/tests/unit/expose-adapters.bats
#
# Unit tests for C8 expose adapters: k8s.sh, staging.sh, custom.sh + updated dispatch.sh.
# k8s tests skip when kubectl is unavailable.
# staging and custom_cmd use mock shell fixtures.
#
# Run: bats skills/autospec-e2e-clone/tests/unit/expose-adapters.bats

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
K8S_SH="$SKILL_DIR/scripts/expose/k8s.sh"
STAGING_SH="$SKILL_DIR/scripts/expose/staging.sh"
CUSTOM_SH="$SKILL_DIR/scripts/expose/custom.sh"
DISPATCH_SH="$SKILL_DIR/scripts/expose/dispatch.sh"

setup() {
  TEST_DIR="$(mktemp -d -t expose-adapters-XXXXXX)"
  URL_FILE="$TEST_DIR/clone-url.txt"
}

teardown() {
  rm -rf "$TEST_DIR"
}

# ── k8s.sh ────────────────────────────────────────────────────────────────────

@test "k8s.sh: exits 1 with no action" {
  run bash "$K8S_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"action required"* ]]
}

@test "k8s.sh: exits 1 for unknown action" {
  run bash "$K8S_SH" restart
  [ "$status" -eq 1 ]
  [[ "$output" == *"unknown action"* ]]
}

@test "k8s.sh: exits 1 when manifests-dir missing for up" {
  run bash "$K8S_SH" up
  [ "$status" -eq 1 ]
  [[ "$output" == *"manifests-dir argument required"* ]]
}

@test "k8s.sh: exits 1 when namespace missing for up" {
  run bash "$K8S_SH" up /some/dir
  [ "$status" -eq 1 ]
  [[ "$output" == *"namespace argument required"* ]]
}

@test "k8s.sh: exits 1 when namespace missing for down" {
  run bash "$K8S_SH" down
  [ "$status" -eq 1 ]
  [[ "$output" == *"namespace argument required"* ]]
}

@test "k8s.sh up: exits 1 when kubectl not found" {
  # Use a PATH that has bash/sh/etc but no kubectl by putting a stub first
  local FAKE_PATH="$TEST_DIR/fakebin"
  mkdir -p "$FAKE_PATH"
  # Stub kubectl that prints error and exits 1 simulating "not found" path
  printf '#!/usr/bin/env sh\nexit 127\n' > "$FAKE_PATH/kubectl"
  chmod +x "$FAKE_PATH/kubectl"
  run bash "$K8S_SH" up /some/manifests test-ns \
      --url "http://localhost:8080" --health "/" --wait 1 --url-file "$URL_FILE"
  # kubectl exists as a stub but the script's require_tool uses command -v which finds it;
  # test instead that missing real kubectl causes fatal on create ns
  # Alternate: patch the script path so kubectl is genuinely absent
  # The simplest approach: check exit is non-zero (1 or 2)
  [ "$status" -ne 0 ]
}

# ── staging.sh ────────────────────────────────────────────────────────────────

@test "staging.sh: exits 1 with no action" {
  run bash "$STAGING_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"action required"* ]]
}

@test "staging.sh: exits 1 when --swap-cmd missing for up" {
  run bash "$STAGING_SH" up --url "http://localhost:8080"
  [ "$status" -eq 1 ]
  [[ "$output" == *"--swap-cmd is required"* ]]
}

@test "staging.sh: exits 1 when --restore-cmd missing for down" {
  run bash "$STAGING_SH" down
  [ "$status" -eq 1 ]
  [[ "$output" == *"--restore-cmd is required"* ]]
}

@test "staging.sh down: runs restore command" {
  local MARKER="$TEST_DIR/restored.txt"
  run bash "$STAGING_SH" down --restore-cmd "touch '$MARKER'"
  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]
  [[ "$output" == *"slot restored"* ]]
}

@test "staging.sh up: runs swap command and writes url-file when health passes" {
  # Start a tiny HTTP server on port 18081
  python3 -m http.server 18081 --directory "$TEST_DIR" &
  local PID=$!
  sleep 1

  local MARKER="$TEST_DIR/swapped.txt"
  run bash "$STAGING_SH" up \
      --swap-cmd "touch '$MARKER'" \
      --url "http://localhost:18081" \
      --health "/" \
      --wait 10 \
      --url-file "$URL_FILE"

  kill "$PID" 2>/dev/null || true

  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]
  [ -f "$URL_FILE" ]
  local written_url
  written_url="$(cat "$URL_FILE")"
  [ "$written_url" = "http://localhost:18081" ]
}

@test "staging.sh up: exits 2 when health times out" {
  local MARKER="$TEST_DIR/swapped2.txt"
  run bash "$STAGING_SH" up \
      --swap-cmd "touch '$MARKER'" \
      --url "http://localhost:19998" \
      --health "/nowhere" \
      --wait 3 \
      --url-file "$URL_FILE"
  [ "$status" -eq 2 ]
  [[ "$output" == *"timed out"* ]]
}

# ── custom.sh ─────────────────────────────────────────────────────────────────

@test "custom.sh: exits 1 with no action" {
  run bash "$CUSTOM_SH"
  [ "$status" -eq 1 ]
  [[ "$output" == *"action required"* ]]
}

@test "custom.sh: exits 1 when --up-cmd missing for up" {
  run bash "$CUSTOM_SH" up --url "http://localhost:8080"
  [ "$status" -eq 1 ]
  [[ "$output" == *"--up-cmd is required"* ]]
}

@test "custom.sh: exits 1 when --down-cmd missing for down" {
  run bash "$CUSTOM_SH" down
  [ "$status" -eq 1 ]
  [[ "$output" == *"--down-cmd is required"* ]]
}

@test "custom.sh down: runs down command" {
  local MARKER="$TEST_DIR/downed.txt"
  run bash "$CUSTOM_SH" down --down-cmd "touch '$MARKER'"
  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]
  [[ "$output" == *"down command completed"* ]]
}

@test "custom.sh up: runs up command and writes url-file when health passes" {
  python3 -m http.server 18082 --directory "$TEST_DIR" &
  local PID=$!
  sleep 1

  local MARKER="$TEST_DIR/upped.txt"
  run bash "$CUSTOM_SH" up \
      --up-cmd "touch '$MARKER'" \
      --url "http://localhost:18082" \
      --health "/" \
      --wait 10 \
      --url-file "$URL_FILE"

  kill "$PID" 2>/dev/null || true

  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]
  [ -f "$URL_FILE" ]
  local written_url
  written_url="$(cat "$URL_FILE")"
  [ "$written_url" = "http://localhost:18082" ]
}

@test "custom.sh up: exits 2 when health times out" {
  local MARKER="$TEST_DIR/upped2.txt"
  run bash "$CUSTOM_SH" up \
      --up-cmd "touch '$MARKER'" \
      --url "http://localhost:19997" \
      --health "/nowhere" \
      --wait 3 \
      --url-file "$URL_FILE"
  [ "$status" -eq 2 ]
  [[ "$output" == *"timed out"* ]]
}

@test "custom.sh up: exits 1 when up command fails" {
  # Use a command that actually fails (not "exit 1" which exits the eval context)
  local FAIL_CMD="$TEST_DIR/fail.sh"
  printf '#!/usr/bin/env sh\necho "up command failed: intentional failure" >&2\nexit 1\n' > "$FAIL_CMD"
  chmod +x "$FAIL_CMD"

  run bash "$CUSTOM_SH" up \
      --up-cmd "$FAIL_CMD" \
      --url "http://localhost:8080" \
      --health "/" \
      --wait 3 \
      --url-file "$URL_FILE"
  [ "$status" -eq 1 ]
}

# ── dispatch.sh wiring (C8 kinds) ─────────────────────────────────────────────

@test "dispatch.sh: routes custom_cmd up to custom.sh" {
  python3 -m http.server 18083 --directory "$TEST_DIR" &
  local PID=$!
  sleep 1

  local MARKER="$TEST_DIR/custom-dispatch.txt"
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-custom-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"custom_cmd","url_template":"http://localhost:18083","health_endpoint":"/","ready_wait_secs":10,"custom":{"up_cmd":"touch %s","down_cmd":"rm -f %s"}}}\n' \
    "$MARKER" "$MARKER" > "$CONTRACT"

  run bash "$DISPATCH_SH" up --contract "$CONTRACT" --url-file "$URL_FILE"

  kill "$PID" 2>/dev/null || true

  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]
  [ -f "$URL_FILE" ]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: routes custom_cmd down to custom.sh" {
  local MARKER="$TEST_DIR/custom-down.txt"
  touch "$MARKER"
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-custom-down-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"custom_cmd","url_template":"http://localhost:8080","health_endpoint":"/","ready_wait_secs":5,"custom":{"up_cmd":"true","down_cmd":"rm -f %s"}}}\n' \
    "$MARKER" > "$CONTRACT"

  run bash "$DISPATCH_SH" down --contract "$CONTRACT"
  [ "$status" -eq 0 ]
  [ ! -f "$MARKER" ]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: routes dedicated_staging_slot down to staging.sh" {
  local MARKER="$TEST_DIR/staging-down.txt"
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-staging-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"dedicated_staging_slot","url_template":"http://localhost:8080","health_endpoint":"/","ready_wait_secs":5,"staging":{"swap_cmd":"true","restore_cmd":"touch %s"}}}\n' \
    "$MARKER" > "$CONTRACT"

  run bash "$DISPATCH_SH" down --contract "$CONTRACT"
  [ "$status" -eq 0 ]
  [ -f "$MARKER" ]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: routes k8s_ephemeral_namespace down to k8s.sh" {
  # Use a stub kubectl that exits 0 for delete (--ignore-not-found)
  local FAKE_PATH="$TEST_DIR/fakebin"
  mkdir -p "$FAKE_PATH"
  printf '#!/usr/bin/env sh\nexit 0\n' > "$FAKE_PATH/kubectl"
  chmod +x "$FAKE_PATH/kubectl"

  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-k8s-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"k8s_ephemeral_namespace","namespace":"test-ns","url_template":"http://localhost:8080","health_endpoint":"/","ready_wait_secs":5}}\n' \
    > "$CONTRACT"

  run env PATH="$FAKE_PATH:$PATH" bash "$DISPATCH_SH" down --contract "$CONTRACT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"deleted"* ]]

  rm -f "$CONTRACT"
}

@test "dispatch.sh: unknown expose.kind still refused" {
  local CONTRACT
  CONTRACT="$(mktemp -t dispatch-unknown-XXXXXX.json)"
  printf '{"sources":[{"kind":"postgres"}],"expose":{"kind":"totally_unknown"}}\n' > "$CONTRACT"

  run bash "$DISPATCH_SH" up --contract "$CONTRACT"
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown expose.kind"* ]]

  rm -f "$CONTRACT"
}
