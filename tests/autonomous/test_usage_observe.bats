#!/usr/bin/env bats
# tests/autonomous/test_usage_observe.bats — unit tests for
# scripts/usage-observe.sh (F6a per-harness live-usage observability probe).
#
# Covers:
#  - each known harness emits valid JSON with the 4 contract keys
#  - the default (no probe) finding is observable:false / percent:null per harness
#  - an unknown harness exits non-zero
#  - the probe seam (mocked as a subprocess) flips observable:true with a percent
#  - a probe emitting an out-of-range / non-numeric value falls back to false
#  - macOS bash 3.2 compatibility: write fixture to a real temp file before [ -f ]

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/scripts/usage-observe.sh"

    TEST_DIR="$(mktemp -d)"
    export TEST_DIR
    # Drop any inherited probe overrides so the default path is deterministic.
    unset AUTOSPEC_USAGE_PROBE_CLAUDE AUTOSPEC_USAGE_PROBE_CODEX AUTOSPEC_USAGE_PROBE_OPENCODE
    # Point the auto-discovered OpenCode probe at a non-existent DB so the
    # default opencode finding stays deterministic (observable:false) here; the
    # auto-discovery success path is covered by its own test with a fixture DB.
    export OPENCODE_DB_PATH="$TEST_DIR/nonexistent-opencode.db"
}

teardown() {
    rm -rf "$TEST_DIR"
}

# Write a mock probe to a REAL temp file, assert it exists before use
# (bash 3.2: never `[ -f <(...) ]`), make it executable, echo its path.
make_probe() {
    value="$1"
    probe="$TEST_DIR/probe.sh"
    printf '#!/usr/bin/env bash\necho "%s"\n' "$value" > "$probe"
    [ -f "$probe" ]
    chmod +x "$probe"
    printf '%s' "$probe"
}

# ── JSON contract ───────────────────────────────────────────────────────────

@test "claude: emits JSON with all four contract keys" {
    run bash "$SCRIPT" claude
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e 'has("harness") and has("observable") and has("percent") and has("source")'
    [ "$(printf '%s' "$output" | jq -r '.harness')" = "claude" ]
}

@test "codex: emits JSON with all four contract keys" {
    run bash "$SCRIPT" codex
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e 'has("harness") and has("observable") and has("percent") and has("source")'
    [ "$(printf '%s' "$output" | jq -r '.harness')" = "codex" ]
}

@test "opencode: emits JSON with all four contract keys" {
    run bash "$SCRIPT" opencode
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e 'has("harness") and has("observable") and has("percent") and has("source")'
    [ "$(printf '%s' "$output" | jq -r '.harness')" = "opencode" ]
}

# ── Default (unobservable) finding ──────────────────────────────────────────

@test "claude: unobservable by default — observable:false, percent:null" {
    run bash "$SCRIPT" claude
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

@test "codex: unobservable by default — observable:false, percent:null" {
    run bash "$SCRIPT" codex
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

@test "opencode: unobservable by default — observable:false, percent:null" {
    run bash "$SCRIPT" opencode
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

# ── Unknown harness ─────────────────────────────────────────────────────────

@test "unknown harness exits non-zero" {
    run bash "$SCRIPT" bogus
    [ "$status" -ne 0 ]
}

@test "missing harness argument exits non-zero" {
    run bash "$SCRIPT"
    [ "$status" -ne 0 ]
}

# ── Probe seam (mocked as a subprocess) ─────────────────────────────────────

@test "probe override flips observable:true with the reported percent" {
    probe="$(make_probe 73)"
    export AUTOSPEC_USAGE_PROBE_CLAUDE="$probe"
    run bash "$SCRIPT" claude
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "73" ]
}

@test "probe override accepts a decimal percent" {
    probe="$(make_probe 12.5)"
    export AUTOSPEC_USAGE_PROBE_CODEX="$probe"
    run bash "$SCRIPT" codex
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "12.5" ]
}

@test "probe emitting an out-of-range value falls back to observable:false" {
    probe="$(make_probe 999)"
    export AUTOSPEC_USAGE_PROBE_CLAUDE="$probe"
    run bash "$SCRIPT" claude
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

@test "probe emitting a non-numeric value falls back to observable:false" {
    probe="$(make_probe nonsense)"
    export AUTOSPEC_USAGE_PROBE_OPENCODE="$probe"
    run bash "$SCRIPT" opencode
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

# Probe that prints a valid-looking percent but exits non-zero must NOT be
# trusted (regression: pipeline-masked exit status).
@test "probe printing a number but exiting non-zero falls back to false" {
    probe="$TEST_DIR/probe.sh"
    printf '#!/usr/bin/env bash\necho 73\nexit 1\n' > "$probe"
    [ -f "$probe" ]
    chmod +x "$probe"
    export AUTOSPEC_USAGE_PROBE_CLAUDE="$probe"
    run bash "$SCRIPT" claude
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "null" ]
}

# A bare "." would coerce to 0 in awk and break jq --argjson; it must be
# rejected as non-numeric (regression: is_valid_percent dot handling).
@test "probe emitting a bare dot falls back to false (valid JSON)" {
    probe="$(make_probe .)"
    export AUTOSPEC_USAGE_PROBE_CODEX="$probe"
    run bash "$SCRIPT" codex
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.observable == false and .percent == null'
}

@test "a probe override on one harness does not affect another" {
    probe="$(make_probe 50)"
    export AUTOSPEC_USAGE_PROBE_CLAUDE="$probe"
    run bash "$SCRIPT" codex
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "false" ]
}

# ── OpenCode SQLite auto-discovery ───────────────────────────────────────────

@test "opencode: auto-discovers the shipped SQLite probe and reports a live percent" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not installed"
    local db="$TEST_DIR/opencode.db"
    python3 - "$db" <<'PY'
import sqlite3, sys, time
conn = sqlite3.connect(sys.argv[1])
conn.execute("CREATE TABLE session (time_updated INTEGER, tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER)")
conn.execute("INSERT INTO session VALUES (?, 200, 200, 100)", (int(time.time() * 1000),))
conn.commit()
PY
    export OPENCODE_DB_PATH="$db"
    export OPENCODE_USAGE_WINDOW_HOURS=24
    export AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000

    run bash "$SCRIPT" opencode
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "50.0" ]
    [ "$(printf '%s' "$output" | jq -r '.source')" = "probe:AUTOSPEC_USAGE_PROBE_OPENCODE" ]
}

@test "opencode: SQLite probe caps the percent at 100" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not installed"
    local db="$TEST_DIR/opencode-over.db"
    python3 - "$db" <<'PY'
import sqlite3, sys, time
conn = sqlite3.connect(sys.argv[1])
conn.execute("CREATE TABLE session (time_updated INTEGER, tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER)")
conn.execute("INSERT INTO session VALUES (?, 5000, 5000, 0)", (int(time.time() * 1000),))
conn.commit()
PY
    export OPENCODE_DB_PATH="$db"
    export AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000

    run bash "$SCRIPT" opencode
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.observable')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.percent')" = "100.0" ]
}
