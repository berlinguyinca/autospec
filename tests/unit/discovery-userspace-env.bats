#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SUT="$REPO_ROOT/skills/autospec-shared/scripts/discovery-userspace-env.sh"
VALIDATOR="$REPO_ROOT/skills/autospec-shared/scripts/validate-trend-signal.sh"

setup() {
  TMP="$(mktemp -d)"
  export AUTOSPEC_TREND_LEDGER="$TMP/ledger.jsonl"

  # Fixture PATH: one stub "tool" that, if ever executed, writes a sentinel
  # file — proves the harvester never invokes discovered binaries.
  BIN_DIR="$TMP/bin"
  mkdir -p "$BIN_DIR"
  EXEC_SENTINEL="$TMP/executed.marker"
  cat > "$BIN_DIR/totally-unknown-tool-xyz" <<EOF
#!/usr/bin/env bash
touch "$EXEC_SENTINEL"
EOF
  chmod +x "$BIN_DIR/totally-unknown-tool-xyz"
  export DISCOVERY_ENV_PATH="$BIN_DIR"

  # Fixture "already integrated" scan root: empty, so nothing on the stub
  # PATH is ever considered integrated.
  SCAN_ROOT="$TMP/scan-root"
  mkdir -p "$SCAN_ROOT/skills"
  export DISCOVERY_ENV_SCAN_ROOT="$SCAN_ROOT/skills"

  # Fixture skills dir (separate from the real repo's skills/).
  export DISCOVERY_ENV_SKILLS_DIR="$SCAN_ROOT/skills"

  # No MCP config by default.
  export DISCOVERY_ENV_MCP_CONFIG="$TMP/does-not-exist.json"

  export DISCOVERY_ENV_NOW="2026-07-08T00:00:00Z"

  CFG_OK="$TMP/cfg-ok.json"
  echo '{"discovery":{"userspace":{"opt_out":false},"rate_limits":{"userspace-env":{"window_seconds":3600,"max_per_window":1000}}}}' > "$CFG_OK"

  CFG_OPT_OUT="$TMP/cfg-opt-out.json"
  echo '{"discovery":{"userspace":{"opt_out":true}}}' > "$CFG_OPT_OUT"
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_TREND_LEDGER DISCOVERY_ENV_PATH DISCOVERY_ENV_SCAN_ROOT \
    DISCOVERY_ENV_SKILLS_DIR DISCOVERY_ENV_MCP_CONFIG DISCOVERY_ENV_NOW
}

@test "script exists and is bash -n clean" {
  [ -f "$SUT" ]
  run bash -n "$SUT"
  [ "$status" -eq 0 ]
}

@test "missing config argument exits non-zero" {
  run bash "$SUT"
  [ "$status" -ne 0 ]
}

@test "missing config file exits non-zero" {
  run bash "$SUT" "$TMP/nope.json"
  [ "$status" -ne 0 ]
}

@test "a detected but unintegrated CLI produces one signal with source=userspace-env" {
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]

  run bash "$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh" --show --json --source userspace-env
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key | contains("totally-unknown-tool-xyz"))] | length')" -eq 1 ]
  [ "$(echo "$output" | jq -r '.[] | select(.norm_key | contains("totally-unknown-tool-xyz")) | .source')" = "userspace-env" ]
}

@test "the script never executes a discovered binary" {
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]
  [ ! -f "$EXEC_SENTINEL" ]
}

@test "opt_out => no ledger writes, exit 0" {
  run bash "$SUT" "$CFG_OPT_OUT"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
}

@test "recurrence bumps on repeat run instead of duplicate append" {
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]

  run bash "$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh" --show --json --source userspace-env
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key | contains("totally-unknown-tool-xyz"))] | length')" -eq 1 ]
  [ "$(echo "$output" | jq -r '.[] | select(.norm_key | contains("totally-unknown-tool-xyz")) | .recurrence')" -eq 2 ]
}

@test "appended records pass validate-trend-signal.sh" {
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]
  [ -f "$AUTOSPEC_TREND_LEDGER" ]

  while IFS= read -r line; do
    [ -n "$line" ] || continue
    tmp="$(mktemp)"
    printf '%s' "$line" > "$tmp"
    run bash "$VALIDATOR" "$tmp"
    rm -f "$tmp"
    [ "$status" -eq 0 ]
  done < "$AUTOSPEC_TREND_LEDGER"
}

@test "an unconfigured/absent MCP config is a clean no-op (no error)" {
  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]
}

@test "a configured MCP server not integrated by this repo produces an mcp-integration-gap signal" {
  MCP_CFG="$TMP/mcp.json"
  echo '{"mcpServers":{"totally-unknown-mcp-server":{"command":"noop"}}}' > "$MCP_CFG"
  export DISCOVERY_ENV_MCP_CONFIG="$MCP_CFG"
  # No CLI stubs this round.
  export DISCOVERY_ENV_PATH="$TMP/empty-bin"
  mkdir -p "$TMP/empty-bin"

  run bash "$SUT" "$CFG_OK"
  [ "$status" -eq 0 ]

  run bash "$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh" --show --json --source userspace-env
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key | contains("totally-unknown-mcp-server"))] | length')" -eq 1 ]
  [ "$(echo "$output" | jq -r '.[] | select(.norm_key | contains("totally-unknown-mcp-server")) | .kind')" = "mcp-integration-gap" ]
}
