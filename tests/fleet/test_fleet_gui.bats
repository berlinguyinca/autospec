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
    # Kill any extra servers registered by multi-process tests (e.g. the flock
    # contention test) so a failure before the test's own kill block cannot leak
    # processes that hold ports.
    local _pid
    for _pid in ${EXTRA_PIDS:-}; do
        kill "$_pid" 2>/dev/null || true
        wait "$_pid" 2>/dev/null || true
    done
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

# raw_get <port> <path> [token-header-value]
# GET without the api_get convenience header. If a third arg is given it is sent
# verbatim as the X-Autospec-Token header value (used to send a WRONG token); if
# omitted, NO token header is sent at all. Prints "<body>\n<http_code>" so a test
# can assert both the status line and the JSON body. Used by the auth (#838) tests
# which must reach the server with bad/absent credentials — the api_* helpers
# always attach the correct token and so cannot exercise auth_ok's reject path.
raw_get() {
    local port="$1" path="$2"
    if [[ $# -ge 3 ]]; then
        curl -s -w '\n%{http_code}' \
            -H "X-Autospec-Token: $3" \
            "http://127.0.0.1:${port}${path}"
    else
        curl -s -w '\n%{http_code}' \
            "http://127.0.0.1:${port}${path}"
    fi
}

# raw_post <port> <path> <json-body> [token-header-value]
# POST counterpart to raw_get. Omit the token arg to send no token header; pass a
# value to send a wrong token. Prints "<body>\n<http_code>".
raw_post() {
    local port="$1" path="$2" body="$3"
    if [[ $# -ge 4 ]]; then
        curl -s -w '\n%{http_code}' -X POST \
            -H "X-Autospec-Token: $4" \
            -H "Content-Type: application/json" \
            -d "$body" \
            "http://127.0.0.1:${port}${path}"
    else
        curl -s -w '\n%{http_code}' -X POST \
            -H "Content-Type: application/json" \
            -d "$body" \
            "http://127.0.0.1:${port}${path}"
    fi
}

# http_code_of / body_of: split the two-line raw_* output.
http_code_of() { printf '%s\n' "$1" | tail -n1; }
body_of()      { printf '%s\n' "$1" | sed '$d'; }

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
# Test 5: flock serializes concurrent POSTs from TWO real server processes (#837)
#
# Why this rewrite: a single fleet-gui-server.py is strictly single-threaded
# (`while not shutdown_event: server.handle_request()`), so two POSTs aimed at
# ONE server are serialized by the HTTP accept loop — never by flock. A test
# that fires both at one server would still pass with fcntl.flock deleted; it
# proves nothing about the lock.
#
# This test creates REAL cross-process contention: two INDEPENDENT server
# processes share the SAME WORKSPACE and the SAME LOCK_FILE, and each receives
# one simultaneous POST. flock is the ONLY thing serializing their
# read-merge-write of autospec-fleet.yml across the two OS processes.
#
# What it asserts (the genuine flock guarantee): each POST adds a DISTINCT
# UNMANAGED key. _handle_config_post does read-modify-write (load existing →
# merge → atomic os.replace). The final write is atomic, so the file is always
# a single intact YAML shape EVEN WITHOUT flock — checking "file is valid YAML"
# would not detect a missing lock. The thing flock actually prevents is the
# LOST UPDATE: without the lock both processes read the same baseline and the
# second writer clobbers the first writer's key. With flock, the second writer
# reads the first's result, so BOTH unmanaged keys survive.
#
# Determinism: a large seeded baseline config widens the read-merge window so
# the unlocked race is reliably triggered (no sleeps/timing in the assertion
# path). Mutation-verified: deleting the two fcntl.flock() calls makes this
# test FAIL (one key is lost), 12/12 runs; with flock it PASSES, 12/12 runs.
# ---------------------------------------------------------------------------
@test "flock serializes concurrent POSTs across two server processes — no lost update (#837)" {
    # Stub gh (servers call it only for /api/repos, not for config POSTs, but
    # keep PATH consistent with the other tests).
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Seed a LARGE baseline config sharing the workspace both servers use.
    # The size makes load_yaml_config (the read half of read-modify-write) take
    # long enough that, with flock removed, the two processes deterministically
    # read the same baseline and the lost update manifests every run. The
    # baseline carries only managed keys; each POST adds its own unmanaged key.
    local config_file="$TMP/autospec-fleet.yml"
    python3 - "$config_file" <<'PY'
import sys
n = 8000
lines = ["version: 1", "workspace: ws", "default_profile: baseline",
         "parallel_repos: 1", "repos:"]
for i in range(n):
    lines.append(f"  - url: https://github.com/org/repo{i}")
    lines.append("    enabled: true")
open(sys.argv[1], "w").write("\n".join(lines) + "\n")
PY

    # Two independent servers on two ports, SHARING $TMP (workspace) + $LOCK_FILE.
    local port_a port_b
    port_a="$(pick_free_port)"
    port_b="$(pick_free_port)"

    IDLE_SECS=3600 PATH="$BIN:$PATH" python3 "$SERVER_PY" \
        "$port_a" "$TOKEN" "$TMP" "$GUI_HTML" "$LOCK_FILE" "0" "3600" \
        >/dev/null 2>&1 &
    local srv_a=$!
    IDLE_SECS=3600 PATH="$BIN:$PATH" python3 "$SERVER_PY" \
        "$port_b" "$TOKEN" "$TMP" "$GUI_HTML" "$LOCK_FILE" "0" "3600" \
        >/dev/null 2>&1 &
    local srv_b=$!
    # Register both PIDs so teardown reaps them even if an assertion below fails
    # before the explicit kill block.
    EXTRA_PIDS="$srv_a $srv_b"

    wait_for_port "$port_a" 50 || { kill "$srv_a" "$srv_b" 2>/dev/null; false; }
    wait_for_port "$port_b" 50 || { kill "$srv_a" "$srv_b" 2>/dev/null; false; }

    # Each POST adds a DISTINCT unmanaged key. Fire them simultaneously, one at
    # each independent server, so only flock can serialize the shared write.
    local resp_a resp_b
    resp_a="$TMP/resp_a.json"
    resp_b="$TMP/resp_b.json"
    api_post "$port_a" "/api/config" '{"experimental_a":1}' > "$resp_a" &
    local pid_a=$!
    api_post "$port_b" "/api/config" '{"experimental_b":2}' > "$resp_b" &
    local pid_b=$!
    wait "$pid_a"
    wait "$pid_b"

    # Both POSTs must have been accepted (saved:true).
    python3 -c "
import json
for f in ('$resp_a', '$resp_b'):
    d = json.loads(open(f).read())
    assert d.get('saved') is True, f'POST did not save: {f} -> {d}'
print('both POSTs saved')
"

    # Let each server's 1s post-save shutdown settle, then reap.
    sleep 2
    kill "$srv_a" "$srv_b" 2>/dev/null || true
    wait "$srv_a" 2>/dev/null || true
    wait "$srv_b" 2>/dev/null || true

    # The on-disk file must be a single intact YAML shape (atomic write) AND —
    # the real flock guarantee — must retain BOTH unmanaged keys. Without flock
    # the second writer clobbers the first writer's baseline read, dropping one
    # of experimental_a / experimental_b: that is the lost update flock prevents.
    [ -f "$config_file" ]
    python3 -c "
import yaml, sys
with open(sys.argv[1]) as f:
    cfg = yaml.safe_load(f)
assert isinstance(cfg, dict), f'not a dict / corrupted write: {type(cfg)}'

# Managed baseline keys must survive the merge.
for k in ('version', 'workspace', 'default_profile', 'parallel_repos', 'repos'):
    assert k in cfg, f'missing managed key {k}'
assert isinstance(cfg['repos'], list), 'repos not a list'

# The lock guarantee: NO lost update — both concurrent writers' keys present.
assert cfg.get('experimental_a') == 1, (
    'lost update: experimental_a missing — flock did not serialize the '
    'read-modify-write across the two server processes')
assert cfg.get('experimental_b') == 2, (
    'lost update: experimental_b missing — flock did not serialize the '
    'read-modify-write across the two server processes')
print('OK — both unmanaged keys survived concurrent cross-process writes')
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

# ---------------------------------------------------------------------------
# Test 7: auth/token enforcement — 401 on missing/wrong token (#838)
#
# The AC "the HTTP server requires the URL token" is enforced solely by
# auth_ok (fleet-gui-server.py): the JSON API routes return
# 401 {"error":"unauthorized"} unless the request carries the correct token via
# the X-Autospec-Token header OR the ?t= query param, and the HTML route
# (_serve_html) re-checks auth_ok and returns a bare 401. Before this test every
# existing test sent the correct token, so auth_ok could be inverted or deleted
# and the suite would stay green.
#
# This test reaches GET /api/repos, GET /api/config, POST /api/config and GET /
# with (a) NO token and (b) a WRONG token, asserting HTTP 401, the
# {"error":"unauthorized"} body on the JSON routes, and — the destructive part —
# that an unauthorized POST writes NO config file. As a positive control it also
# confirms the ?t= query-param path authenticates a GET, so the assertions pin
# auth_ok's real accept/reject contract rather than a blanket "everything 401s".
#
# Mutation-verified (see commit): with a copy of the server whose auth_ok is
# forced to `return True`, every reject assertion here FAILS (routes answer 200
# and the POST writes the file); against the real server they all PASS. So this
# test genuinely fails when auth is broken — it cannot pass while real auth fails.
# ---------------------------------------------------------------------------
@test "auth enforcement — missing/wrong token yields 401 and no write (#838)" {
    local port
    port="$(pick_free_port)"

    # Stub gh so the server starts (and so a 401 on /api/repos is provably the
    # auth gate, not a missing-gh 503: with auth disabled this stub would let the
    # route return 200, which is exactly what the mutation check relies on).
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Ensure no config file exists going in, so a POST that is (wrongly) accepted
    # would create one — the absence check below is then meaningful.
    local config_file="$TMP/autospec-fleet.yml"
    rm -f "$config_file"

    PATH="$BIN:$PATH" start_server "$port"

    local resp code body
    local wrong="not-the-token"

    # --- GET /api/repos: no token, then wrong token -> 401 unauthorized ----
    resp="$(raw_get "$port" "/api/repos")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]

    resp="$(raw_get "$port" "/api/repos" "$wrong")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]

    # --- GET /api/config: no token, then wrong token -> 401 unauthorized ---
    resp="$(raw_get "$port" "/api/config")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]

    resp="$(raw_get "$port" "/api/config" "$wrong")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]

    # --- POST /api/config: no token, then wrong token -> 401 AND no file ---
    local post_body='{"version":1,"workspace":"x","default_profile":"p","parallel_repos":1,"repos":[]}'

    resp="$(raw_post "$port" "/api/config" "$post_body")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]
    # An unauthorized POST must NOT have written the config file.
    [ ! -f "$config_file" ]

    resp="$(raw_post "$port" "/api/config" "$post_body" "$wrong")"
    code="$(http_code_of "$resp")"; body="$(body_of "$resp")"
    [ "$code" = "401" ]
    [[ "$body" == *'"error": "unauthorized"'* ]]
    [ ! -f "$config_file" ]

    # --- GET / (HTML route): no token, then wrong token -> bare 401 --------
    # _serve_html re-checks auth_ok and replies 401 with no JSON body.
    resp="$(raw_get "$port" "/")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]

    resp="$(raw_get "$port" "/" "$wrong")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]

    # --- Positive control: the ?t= query-param path DOES authenticate. -----
    # This pins auth_ok's accept branch too, so the test asserts the real
    # accept/reject contract rather than "every request 401s" (which would still
    # pass if auth_ok were hard-wired to reject everything).
    resp="$(raw_get "$port" "/api/config?t=$TOKEN")"
    code="$(http_code_of "$resp")"
    [ "$code" = "200" ]
}

# ---------------------------------------------------------------------------
# Test 8: --once smoke mode actually starts the server and hits both endpoints (#839)
#
# Per spec § Primary smoke test, `fleet-gui.sh --no-browser --print-url --once`
# must "launch the server, hit /api/repos and /api/config once, exit 0" — an
# end-to-end guarantee that the HTTP surface is alive. The shipped --once branch
# previously only re-bound the port to confirm bindability and returned BEFORE
# exec'ing the server, so the server never started, no request was issued, and
# the server's own ONCE shutdown handling was unreachable dead code. The
# documented smoke command proved nothing about the live HTTP surface.
#
# This test runs the real launcher in --once mode against a real (stubbed-gh)
# server over real HTTP and asserts:
#   - exit 0
#   - the URL is printed (--print-url)
#   - BOTH /api/repos and /api/config returned HTTP 200 (the launcher reports
#     each endpoint's status; the test pins both to 200)
#   - the gh stub was invoked, proving /api/repos genuinely reached the handler
#     (which shells out to `gh repo list`) rather than being short-circuited.
#
# Mutation-verified: reverting --once to the old port-bindable-only branch (which
# never starts the server) makes this test FAIL — no smoke line is emitted, the
# gh stub is never called, and the endpoint statuses are absent. So the test
# cannot pass unless --once truly boots the server and exercises both endpoints.
# ---------------------------------------------------------------------------
@test "--once smoke mode boots the server and hits /api/repos + /api/config (#839)" {
    # Stub gh so /api/repos returns a real 200 (and so we can prove the route was
    # actually reached: the handler shells out to `gh repo list`, and this stub
    # records each invocation to a marker file).
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    local gh_calls="$TMP/gh-calls.log"
    cat > "$BIN/gh" <<EOF
#!/usr/bin/env bash
echo "called \$*" >> "$gh_calls"
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Run the launcher in --once smoke mode from within the workspace tmpdir, so
    # /api/config reads $TMP/autospec-fleet.yml (absent → default skeleton, 200).
    run bash -c "cd '$TMP' && PATH=\"$BIN:\$PATH\" AUTOSPEC_GUI_IDLE_SECS=30 bash '$GUI_SH' --no-browser --print-url --once 2>&1"

    # Exit 0 — the documented smoke guarantee.
    [ "$status" -eq 0 ]

    # --print-url must have emitted the tokenized URL.
    [[ "$output" == *"http://127.0.0.1:"*"/?t="* ]]

    # Both endpoints must have been hit and returned 200. The launcher reports
    # each endpoint's HTTP status in its smoke output.
    [[ "$output" == *"/api/repos"*"200"* ]]
    [[ "$output" == *"/api/config"*"200"* ]]

    # The gh stub must have been invoked at least once, proving /api/repos
    # genuinely reached the server handler (not short-circuited by a
    # port-bindable-only branch that never starts the server).
    [ -f "$gh_calls" ]
    grep -q "repo list" "$gh_calls"
}

# ---------------------------------------------------------------------------
# Test 9: --once must not open a real browser even without --no-browser (#839)
#
# In --once mode the server self-shuts-down once both /api/repos and /api/config
# have been served. If a real browser were opened, its GUI JS would hit those two
# endpoints first, satisfying the ONCE gate and tearing the server down before
# the smoke client issues its own GETs — a flaky premature-shutdown race. So
# --once must suppress browser opening regardless of --no-browser. This test runs
# --once WITHOUT --no-browser, stubbing xdg-open/open to record any invocation,
# and asserts the browser was never opened while the smoke still exits 0.
#
# Mutation-verified: gating browser-open on NO_BROWSER alone (dropping the ONCE
# guard) makes the browser stub fire and this test FAIL.
# ---------------------------------------------------------------------------
@test "pick_port exhaustion — exits 1 with clear message and code_health sentinel (#840)" {
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Create a wrapper script that calls fleet-gui.sh but injects a pick_port
    # override via a PICK_PORT env var. We patch fleet-gui.sh's pick_port by
    # pre-binding all ports in the range — instead, the simplest approach is to
    # create a fleet-gui-wrapper.sh that sources a temporary override.
    #
    # Since fleet-gui.sh calls main at bottom and is not sourceable, we create a
    # tiny helper that invokes fleet-gui.sh via env trickery: override PICK_PORT_CMD
    # to a script that prints nothing. But fleet-gui.sh calls python3 directly.
    #
    # Cleanest verified approach: write a python3 shim that prints nothing for
    # the pick_port invocation and passes through all others. The pick_port call
    # is uniquely identified by "-c" arg containing "socket" AND "random.randint".
    cat > "$BIN/python3" <<'PYEOF'
#!/usr/bin/env bash
# Shim: intercept pick_port's specific -c invocation (socket + random.randint),
# print nothing to simulate all 20 ports busy.  All other python3 calls (like
# pick_token, fleet-gui-server.py etc.) are forwarded to the real interpreter.
joined="${*}"
if [[ "$joined" == *"random.randint"* && "$joined" == *"socket"* ]]; then
    # Simulate exhaustion: print nothing and exit 0
    exit 0
fi
exec /usr/bin/python3 "$@"
PYEOF
    chmod +x "$BIN/python3"

    # fleet-gui.sh should detect the empty port, print the error, and exit 1.
    # Use timeout to guard against any unexpected hang.
    run bash -c "cd '$TMP' && PATH=\"$BIN:\$PATH\" timeout 10 bash '$GUI_SH' --no-browser 2>&1"

    # Must exit non-zero (timeout=124 also acceptable but we expect 1)
    [ "$status" -ne 0 ]
    [ "$status" -ne 124 ]
    # Must emit a human-readable message on stderr (captured via 2>&1)
    [[ "$output" == *"no free port"* ]]
    # Must emit the code_health sentinel (matches the missing-gh pattern)
    [[ "$output" == *"code_health:fleet_gui_no_free_port"* ]]
}

@test "--once suppresses browser open even without --no-browser (#839)" {
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    local browser_calls="$TMP/browser-calls.log"

    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Stub BOTH browser openers to record any invocation. open_browser tries
    # xdg-open first, then open; either firing means a real browser was launched.
    for opener in xdg-open open; do
        cat > "$BIN/$opener" <<EOF
#!/usr/bin/env bash
echo "opened \$*" >> "$browser_calls"
EOF
        chmod +x "$BIN/$opener"
    done

    # --once WITHOUT --no-browser. Put the stub BIN first on PATH so the stub
    # openers win over any real ones.
    run bash -c "cd '$TMP' && PATH=\"$BIN:\$PATH\" AUTOSPEC_GUI_IDLE_SECS=30 bash '$GUI_SH' --print-url --once 2>&1"

    # Smoke still succeeds end-to-end.
    [ "$status" -eq 0 ]
    [[ "$output" == *"/api/repos"*"200"* ]]
    [[ "$output" == *"/api/config"*"200"* ]]

    # The browser must NOT have been opened in --once mode.
    [ ! -f "$browser_calls" ]
}
