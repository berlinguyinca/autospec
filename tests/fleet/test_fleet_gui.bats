#!/usr/bin/env bats
# tests/fleet/test_fleet_gui.bats — backend tests for the fleet GUI server.
#
# Covers (per spec 2026-06-01-autospec-fleet-gui-design.md § Tests required):
#   1. backend sort order — pushedAt desc
#   2. config round-trip — unmanaged keys preserved
#   3. missing gh — exit 1 + code_health:fleet_gui_missing_gh on stderr
#   4. default skeleton — GET /api/config when autospec-fleet.yml absent
#   5. flock serialization — 2 concurrent POSTs produce one valid full shape

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

# pick_free_port: print an available TCP port on 127.0.0.1
pick_free_port() {
    python3 -c "
import socket
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', 0))
p = s.getsockname()[1]
s.close()
print(p)
"
}

# wait_for_port <port> [max_tries]
wait_for_port() {
    local port="$1"
    local tries="${2:-30}"
    local i
    for (( i=0; i<tries; i++ )); do
        python3 -c "
import socket, sys
s = socket.socket()
s.settimeout(0.2)
try:
    s.connect(('127.0.0.1', $port))
    s.close()
    sys.exit(0)
except Exception:
    sys.exit(1)
" 2>/dev/null && return 0
        sleep 0.1
    done
    return 1
}

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SERVER_PY="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-gui-server.py"
    GUI_SH="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-gui.sh"
    GUI_HTML="$REPO_ROOT/skills/autospec-fleet/gui/index.html"

    # Each test gets its own workspace tmpdir
    TMP="$(mktemp -d -t fleet-gui-test.XXXXXX)"

    # Shared token for tests that start a server
    TOKEN="testtoken99"

    # Lock file location mirrors what fleet-gui-server.py expects
    LOCK_FILE="$TMP/.autospec-fleet/.gui-lock"

    # No background server PID by default
    GUI_PID=""
}

teardown() {
    # Kill any server started by a test
    if [[ -n "${GUI_PID:-}" ]]; then
        kill "$GUI_PID" 2>/dev/null || true
        wait "$GUI_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP"
}

# start_server <port>: start the fleet-gui-server in background, sets GUI_PID
# Honors an exported PYTHONPATH (used by the no-PyYAML test to shim out yaml).
start_server() {
    local port="$1"
    IDLE_SECS="${IDLE_SECS:-3600}"
    python3 "$SERVER_PY" \
        "$port" "$TOKEN" "$TMP" "$GUI_HTML" "$LOCK_FILE" "0" "$IDLE_SECS" \
        >/dev/null 2>&1 &
    GUI_PID=$!
    wait_for_port "$port" 30 || {
        echo "server did not start on port $port" >&2
        return 1
    }
}

# api_get <port> <path>: GET with token header; print response body
api_get() {
    local port="$1" path="$2"
    curl -s -H "X-Autospec-Token: $TOKEN" "http://127.0.0.1:${port}${path}"
}

# api_post <port> <path> <json-body>: POST with token header; print response body
api_post() {
    local port="$1" path="$2" body="$3"
    curl -s -X POST \
        -H "X-Autospec-Token: $TOKEN" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "http://127.0.0.1:${port}${path}"
}

# ---------------------------------------------------------------------------
# Test 1: backend sort order — pushedAt desc
# ---------------------------------------------------------------------------
@test "backend sort order — repos returned in pushedAt desc order" {
    local port
    port="$(pick_free_port)"

    # Stub gh: returns 3 repos with out-of-order pushedAt timestamps
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
# Minimal stub: only handles `gh repo list --json ...`
cat <<'JSON'
[
  {"nameWithOwner":"org/repo-a","pushedAt":"2026-06-01T10:00:00Z","visibility":"PUBLIC","description":"newest","url":"https://github.com/org/repo-a"},
  {"nameWithOwner":"org/repo-c","pushedAt":"2026-05-28T08:00:00Z","visibility":"PUBLIC","description":"oldest","url":"https://github.com/org/repo-c"},
  {"nameWithOwner":"org/repo-b","pushedAt":"2026-05-30T12:00:00Z","visibility":"PUBLIC","description":"middle","url":"https://github.com/org/repo-b"}
]
JSON
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    local resp
    resp="$(api_get "$port" "/api/repos")"

    # Extract pushedAt values in order from JSON array
    local first second third
    first="$(python3 -c "import json,sys; d=json.loads(sys.argv[1]); print(d[0]['pushedAt'])" "$resp")"
    second="$(python3 -c "import json,sys; d=json.loads(sys.argv[1]); print(d[1]['pushedAt'])" "$resp")"
    third="$(python3 -c "import json,sys; d=json.loads(sys.argv[1]); print(d[2]['pushedAt'])" "$resp")"

    # Verify descending order: 2026-06-01 > 2026-05-30 > 2026-05-28
    [[ "$first"  == "2026-06-01T10:00:00Z" ]]
    [[ "$second" == "2026-05-30T12:00:00Z" ]]
    [[ "$third"  == "2026-05-28T08:00:00Z" ]]
}

# ---------------------------------------------------------------------------
# Test 2: config round-trip — unmanaged key preserved
# ---------------------------------------------------------------------------
@test "config round-trip preserves unmanaged key experimental_thing: 42" {
    local port
    port="$(pick_free_port)"

    # Stub gh so the server can start (needed by require_gh in fleet-gui.sh,
    # but not called by fleet-gui-server.py directly; we skip gh stub here
    # since fleet-gui-server.py only calls gh for /api/repos, not config).
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    # POST a config with managed keys + one unmanaged key
    local post_body
    post_body='{"version":1,"workspace":".autospec-fleet/repos","default_profile":"qwen3-32b-laptop","parallel_repos":2,"repos":[],"experimental_thing":42}'

    local post_resp
    post_resp="$(api_post "$port" "/api/config" "$post_body")"

    # POST should succeed
    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
assert d.get('saved') == True, f'expected saved=true, got: {d}'
" "$post_resp"

    # Wait for the server to finish the post-save 1-second shutdown timer
    # (the server arms a 1-second delay before shutdown after config POST)
    sleep 1.5
    GUI_PID=""  # server is now stopped

    # Read the on-disk YAML — must contain experimental_thing: 42
    local config_file="$TMP/autospec-fleet.yml"
    [ -f "$config_file" ]

    # Verify unmanaged key survived
    python3 -c "
import yaml, sys
with open(sys.argv[1]) as f:
    cfg = yaml.safe_load(f)
assert cfg.get('experimental_thing') == 42, f'unmanaged key missing or wrong: {cfg}'
assert cfg.get('version') == 1
assert cfg.get('workspace') == '.autospec-fleet/repos'
" "$config_file"

    # Now start a new server on a new port to verify GET round-trips correctly
    local port2
    port2="$(pick_free_port)"
    GUI_PID=""
    PATH="$BIN:$PATH" start_server "$port2"

    local get_resp
    get_resp="$(api_get "$port2" "/api/config")"

    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
assert d.get('exists') == True, f'expected exists=true, got: {d}'
cfg = d.get('config', {})
assert cfg.get('experimental_thing') == 42, f'round-trip: unmanaged key missing: {cfg}'
assert cfg.get('version') == 1
" "$get_resp"
}

# ---------------------------------------------------------------------------
# Test 3: missing gh — exit 1 + code_health:fleet_gui_missing_gh on stderr
# ---------------------------------------------------------------------------
@test "missing gh emits code_health:fleet_gui_missing_gh on stderr and exits 1" {
    # Place a `gh` stub that is NOT executable (mode 000) so `command -v gh`
    # finds the file but cannot execute it — but actually the simplest and most
    # portable approach is a PATH that genuinely has no `gh` binary.
    # We build a minimal PATH: a single temp dir with python3 + bash symlinked in
    # but no gh, so `command -v gh` reliably fails on any system.
    local bin_dir
    bin_dir="$(mktemp -d -t no-gh-bin.XXXXXX)"
    ln -s "$(command -v python3)" "$bin_dir/python3" 2>/dev/null || true
    ln -s "$(command -v bash)"    "$bin_dir/bash"    2>/dev/null || true
    # Deliberately do NOT place gh in bin_dir.

    run bash -c "PATH='$bin_dir' bash '$GUI_SH' --no-browser --print-url --once 2>&1"
    rm -rf "$bin_dir"

    # Must exit non-zero
    [ "$status" -ne 0 ]

    # Must emit the sentinel string
    [[ "$output" == *"code_health:fleet_gui_missing_gh"* ]]
}

# ---------------------------------------------------------------------------
# Test 4: default skeleton when autospec-fleet.yml absent
# ---------------------------------------------------------------------------
@test "default skeleton returned when autospec-fleet.yml is absent" {
    local port
    port="$(pick_free_port)"

    # Ensure no config file exists in TMP
    rm -f "$TMP/autospec-fleet.yml"

    # Stub gh
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    local resp
    resp="$(api_get "$port" "/api/config")"

    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
assert d.get('exists') == False, f'expected exists=false, got: {d}'
cfg = d.get('config', {})
assert cfg.get('version') == 1, f'missing version: {cfg}'
assert 'workspace' in cfg, f'missing workspace: {cfg}'
assert 'default_profile' in cfg, f'missing default_profile: {cfg}'
assert isinstance(cfg.get('repos'), list), f'repos not a list: {cfg}'
" "$resp"
}

# ---------------------------------------------------------------------------
# Test 5: flock serialization — 2 concurrent POSTs produce one valid shape
# ---------------------------------------------------------------------------
@test "flock serializes concurrent POSTs — result is one valid full YAML shape" {
    local port
    port="$(pick_free_port)"

    # Stub gh
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Use a long idle timeout so server does not shut down mid-test
    IDLE_SECS=3600 PATH="$BIN:$PATH" start_server "$port"

    # Two distinct valid payloads
    local body_a body_b
    body_a='{"version":1,"workspace":".autospec-fleet/repos","default_profile":"profile-a","parallel_repos":1,"repos":[{"url":"https://github.com/org/repo-a","enabled":true}]}'
    body_b='{"version":1,"workspace":".autospec-fleet/repos","default_profile":"profile-b","parallel_repos":3,"repos":[{"url":"https://github.com/org/repo-b","enabled":true}]}'

    # Fire both POSTs concurrently; write responses to files (subshell output
    # capture via variable assignment runs in a subshell and cannot propagate
    # back to the parent shell, so we use temp files instead).
    local resp_file_a resp_file_b
    resp_file_a="$TMP/resp_a.json"
    resp_file_b="$TMP/resp_b.json"
    api_post "$port" "/api/config" "$body_a" > "$resp_file_a" &
    local pid_a=$!
    api_post "$port" "/api/config" "$body_b" > "$resp_file_b" &
    local pid_b=$!
    wait "$pid_a"
    wait "$pid_b"

    # At least one POST must have returned saved:true
    python3 -c "
import json, os
for f in ('$resp_file_a', '$resp_file_b'):
    if os.path.exists(f) and os.path.getsize(f) > 0:
        d = json.loads(open(f).read())
        if d.get('saved'):
            print('at least one POST succeeded:', f)
            break
else:
    raise AssertionError('no POST returned saved:true')
"

    # Server does a 1-second post-save shutdown; give it time to settle
    sleep 2
    GUI_PID=""

    # The on-disk file must exist and be valid YAML (no partial / corrupted write)
    local config_file="$TMP/autospec-fleet.yml"
    [ -f "$config_file" ]

    python3 -c "
import yaml, sys
with open(sys.argv[1]) as f:
    cfg = yaml.safe_load(f)

assert isinstance(cfg, dict), f'not a dict: {cfg}'

# Must be one of the two valid full shapes
profile = cfg.get('default_profile')
assert profile in ('profile-a', 'profile-b'), f'unexpected profile: {profile}'

# Required keys all present
for k in ('version', 'workspace', 'default_profile', 'parallel_repos', 'repos'):
    assert k in cfg, f'missing key {k}: {cfg}'

assert isinstance(cfg['repos'], list)
print('OK — profile:', profile)
" "$config_file"
}

# ---------------------------------------------------------------------------
# Test 6: no-PyYAML fallback must NOT destroy unmanaged keys on save (#836)
#
# The spec's autonomous assumption is "Python 3 stdlib only — no pip packages",
# so PyYAML may be absent at runtime. When it is, load_yaml_config must still
# parse a real YAML file (not fall through to json.load and silently return {}),
# otherwise _merge_config drops every unmanaged on-disk key during a save.
#
# This test forces `import yaml` to fail (a PYTHONPATH shim raising ImportError),
# seeds a real YAML config carrying an unmanaged key, POSTs a managed-only
# update, and asserts the unmanaged key survives the save round-trip. It FAILS
# against the buggy json-fallback implementation and PASSES once load/dump are
# stdlib-correct without PyYAML.
# ---------------------------------------------------------------------------
@test "no-PyYAML fallback preserves unmanaged keys on save (#836)" {
    local port
    port="$(pick_free_port)"

    # Shim that makes `import yaml` raise ImportError inside the server process.
    local YAMLSTUB="$TMP/yamlstub"
    mkdir -p "$YAMLSTUB"
    cat > "$YAMLSTUB/yaml.py" <<'EOF'
raise ImportError("PyYAML forced unavailable for test")
EOF

    # Stub gh so the server starts cleanly.
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Seed a REAL YAML config on disk with managed + unmanaged keys.
    # experimental_thing is unmanaged and must survive the save.
    local config_file="$TMP/autospec-fleet.yml"
    cat > "$config_file" <<'EOF'
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-32b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/seeded-repo
    enabled: true
experimental_thing: 42
EOF

    # Start the server with yaml shimmed out — its `import yaml` must fail.
    PYTHONPATH="$YAMLSTUB" PATH="$BIN:$PATH" start_server "$port"

    # POST an update touching only managed keys (no experimental_thing).
    local post_body
    post_body='{"version":1,"workspace":".autospec-fleet/repos","default_profile":"changed-profile","parallel_repos":4,"repos":[]}'

    local post_resp
    post_resp="$(api_post "$port" "/api/config" "$post_body")"

    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
assert d.get('saved') == True, f'expected saved=true, got: {d}'
" "$post_resp"

    # Wait out the post-save shutdown timer.
    sleep 1.5
    GUI_PID=""

    [ -f "$config_file" ]

    # Verify WITHOUT PyYAML (raw text scan): the unmanaged key must still be on
    # disk, and the managed update must have applied. A bug that returns {} on
    # load would have dropped experimental_thing entirely.
    grep -Eq '^experimental_thing:[[:space:]]*42$' "$config_file"
    grep -Eq '^default_profile:[[:space:]]*changed-profile$' "$config_file"

    # GET round-trip, still without PyYAML: the stdlib reader must return the
    # unmanaged key AND parse the empty repos list as [] (not null), so the GUI
    # renders a real list. Start a fresh server with yaml still shimmed out.
    local port2
    port2="$(pick_free_port)"
    GUI_PID=""
    PYTHONPATH="$YAMLSTUB" PATH="$BIN:$PATH" start_server "$port2"

    local get_resp
    get_resp="$(api_get "$port2" "/api/config")"

    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
assert d.get('exists') == True, f'expected exists=true, got: {d}'
cfg = d.get('config', {})
assert cfg.get('experimental_thing') == 42, f'unmanaged key lost on GET: {cfg}'
assert cfg.get('default_profile') == 'changed-profile', f'managed update lost: {cfg}'
assert cfg.get('repos') == [], f'empty repos should round-trip as [] not {cfg.get(\"repos\")!r}: {cfg}'
" "$get_resp"
}
