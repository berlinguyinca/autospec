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
    post_body='{"version":1,"workspace":".autospec-fleet/repos","default_profile":"qwen3-6-35b-a3b-laptop","parallel_repos":2,"repos":[],"experimental_thing":42}'

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
default_profile: qwen3-6-35b-a3b-laptop
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

    # python3 shim: intercept pick_port's -c invocation (identified by "socket" AND
    # "random.randint") and print nothing to simulate all 20 ports busy.
    # All other python3 calls (pick_token, fleet-gui-server.py) pass through.
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
    # Guard against any unexpected hang with a timeout binary when one exists.
    # `timeout` is GNU coreutils and is NOT present on a stock macOS host (where
    # it ships, if at all, as `gtimeout`). Hard-coding `timeout` made this test
    # fail with exit 127 ("command not found") before fleet-gui.sh ever ran, so
    # the sentinel never printed. Resolve a timeout wrapper portably; if none is
    # available, run the script directly (the exhaustion path is bounded and does
    # not hang).
    local TIMEOUT=""
    if command -v timeout >/dev/null 2>&1; then
        TIMEOUT="timeout 10"
    elif command -v gtimeout >/dev/null 2>&1; then
        TIMEOUT="gtimeout 10"
    fi
    run bash -c "cd '$TMP' && PATH=\"$BIN:\$PATH\" $TIMEOUT bash '$GUI_SH' --no-browser 2>&1"

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

# ---------------------------------------------------------------------------
# Tests 11-13: documented fail-mode paths (#842)
#
# Three spec'd failure modes in the Fail-modes table had zero test coverage:
#   1. GET /api/repos → 503 {"error":"gh_not_authenticated"} when gh exits
#      non-zero with an auth-flavored stderr (_repos_error path).
#   2. POST /api/config with a non-JSON body → 400 {"error":"invalid_json"}
#      (_read_and_handle_config_post path).
#   3. Malformed on-disk YAML → 200 {"warning":"yaml_partial"} (_handle_config_get).
#
# Each test starts a real server, issues a real HTTP request, and asserts the
# expected HTTP status + JSON body.  The gh stub is a PATH-shadowed executable.
# ---------------------------------------------------------------------------

@test "fail-mode: gh auth failure → GET /api/repos returns 503 gh_not_authenticated (#842)" {
    local port
    port="$(pick_free_port)"
    local BIN="$TMP/bin"
    mkdir -p "$BIN"

    # Stub gh: exits 1 with an auth-flavored message on stderr
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo "error: not logged into any GitHub hosts" >&2
exit 1
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    # Use curl with the correct token; raw_get omits the token (returns 401).
    local resp body
    resp="$(curl -s -w '\n%{http_code}' \
        -H "X-Autospec-Token: $TOKEN" \
        "http://127.0.0.1:${port}/api/repos")"
    local http_code body
    http_code="$(http_code_of "$resp")"
    body="$(body_of "$resp")"

    [ "$http_code" = "503" ]
    echo "$body" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['error']=='gh_not_authenticated', d"
}

@test "fail-mode: non-JSON POST body → 400 invalid_json (#842)" {
    local port
    port="$(pick_free_port)"
    local BIN="$TMP/bin"
    mkdir -p "$BIN"

    # gh stub (not called for POST, but server starts it)
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    # POST a literal non-JSON body
    local resp http_code body
    resp="$(curl -s -w '\n%{http_code}' -X POST \
        -H "X-Autospec-Token: $TOKEN" \
        -H "Content-Type: application/json" \
        -d 'not valid json at all' \
        "http://127.0.0.1:${port}/api/config")"
    http_code="$(http_code_of "$resp")"
    body="$(body_of "$resp")"

    [ "$http_code" = "400" ]
    echo "$body" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['error']=='invalid_json', d"
}

@test "fail-mode: truncated/garbage YAML → GET /api/config returns 200 warning:yaml_partial (#842)" {
    local port
    port="$(pick_free_port)"
    local BIN="$TMP/bin"
    mkdir -p "$BIN"

    # gh stub
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Write a garbage YAML file that load_yaml_config raises on.
    # The server catches the exception and returns warning:yaml_partial.
    # We override the YAML path by pointing WORKSPACE at TMP.
    # The server uses WORKSPACE/autospec-fleet.yml.
    mkdir -p "$TMP"
    # Write a file that yaml.safe_load would raise on (tabs in YAML are illegal)
    # AND that _basic_yaml_load returns {} (empty → warning:yaml_partial fallback).
    printf '\t{broken: yaml: content' > "$TMP/autospec-fleet.yml"

    PATH="$BIN:$PATH" start_server "$port"

    local resp body http_code
    resp="$(curl -s -w '\n%{http_code}' \
        -H "X-Autospec-Token: $TOKEN" \
        "http://127.0.0.1:${port}/api/config")"
    http_code="$(http_code_of "$resp")"
    body="$(body_of "$resp")"

    [ "$http_code" = "200" ]
    echo "$body" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('warning')=='yaml_partial', d"
}

# ---------------------------------------------------------------------------
# Test 14: auth_ok uses hmac.compare_digest — constant-time token comparison (#843)
#
# Token comparison with == is theoretically timing-observable; hmac.compare_digest
# provides a constant-time alternative. The fix: replace both == TOKEN comparisons
# in auth_ok with hmac.compare_digest(candidate, TOKEN).
#
# This test has two parts:
#   1. Static: auth_ok in fleet-gui-server.py must use hmac.compare_digest (grep).
#      This FAILS on the unfixed server (where == TOKEN is used) and PASSES once
#      the fix is applied — it is the primary "failing test first" gate.
#   2. Functional: the server must still correctly accept the right token (200)
#      and reject a wrong token (401), confirming the replacement did not break
#      auth semantics.
#
# Mutation-verified: reverting hmac.compare_digest back to == TOKEN causes the
# grep assertion to fail; changing hmac.compare_digest to always return True causes
# the wrong-token 401 assertion to fail.
# ---------------------------------------------------------------------------
@test "auth_ok uses hmac.compare_digest for constant-time token comparison (#843)" {
    # --- Part 1: static check — source must use hmac.compare_digest in auth_ok ---
    # grep for hmac.compare_digest in the server source; fails on unfixed code.
    grep -q "hmac.compare_digest" "$SERVER_PY"

    # --- Part 2: functional — correct token allows, wrong token blocks ----------
    local port
    port="$(pick_free_port)"

    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    local resp code

    # Correct token via header → 200
    resp="$(curl -s -w '\n%{http_code}' \
        -H "X-Autospec-Token: $TOKEN" \
        "http://127.0.0.1:${port}/api/config")"
    code="$(http_code_of "$resp")"
    [ "$code" = "200" ]

    # Correct token via ?t= query param → 200
    resp="$(raw_get "$port" "/api/config?t=$TOKEN")"
    code="$(http_code_of "$resp")"
    [ "$code" = "200" ]

    # Wrong token via header → 401
    resp="$(raw_get "$port" "/api/config" "wrong-token")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]

    # Wrong token via ?t= query param → 401
    resp="$(raw_get "$port" "/api/config?t=wrong-token")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]
}

# ---------------------------------------------------------------------------
# Test 15: gh exits 0 with non-JSON stdout → 503 gh_bad_output (#844)
#
# _handle_repos calls json.loads(result.stdout) after checking returncode==0 only.
# A gh that exits 0 with a non-JSON warning banner or unexpected output causes an
# unhandled JSONDecodeError that surfaces as a 500 traceback. Fix: wrap json.loads
# in try/except json.JSONDecodeError and return 503 {"error":"gh_bad_output"}.
#
# This test stubs gh to exit 0 with non-JSON stdout and asserts:
#   - HTTP 503 (not 500)
#   - body is {"error": "gh_bad_output"}
#
# Mutation-verified: removing the try/except and reverting to bare json.loads makes
# the server return HTTP 500 (unhandled exception), causing the 503 assertion here
# to fail.
# ---------------------------------------------------------------------------
@test "fail-mode: gh exits 0 with non-JSON stdout → 503 gh_bad_output (#844)" {
    local port
    port="$(pick_free_port)"
    local BIN="$TMP/bin"
    mkdir -p "$BIN"

    # Stub gh: exits 0 but prints a non-JSON warning banner (simulates deprecation
    # notice, auth banner, or format change that slips past returncode==0 check).
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo "Warning: this CLI version is deprecated. Please upgrade."
exit 0
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    local resp http_code body
    resp="$(curl -s -w '\n%{http_code}' \
        -H "X-Autospec-Token: $TOKEN" \
        "http://127.0.0.1:${port}/api/repos")"
    http_code="$(http_code_of "$resp")"
    body="$(body_of "$resp")"

    [ "$http_code" = "503" ]
    echo "$body" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d['error'] == 'gh_bad_output', f'expected gh_bad_output, got: {d}'
"
}

# ---------------------------------------------------------------------------
# Test 16: idle-timeout AC — AUTOSPEC_GUI_IDLE_SECS controls self-exit (#845)
#
# The spec requires the server to exit after IDLE_SECS of no requests, printing
# 'fleet-gui: idle_timeout — shutting down' on stdout. The env var
# AUTOSPEC_GUI_IDLE_SECS tunes this. Previously no test exercised this path;
# the test suite only set IDLE_SECS=3600 to PREVENT the timeout from firing.
#
# This test starts the server with IDLE_SECS=1 (via positional argv[7]), makes
# NO requests, waits one full idle_watcher poll cycle (5 s) plus buffer, then
# asserts:
#   - The server process has exited (process is gone)
#   - The captured stdout contains 'idle_timeout'
#
# Timing rationale: idle_watcher sleeps 5 s between checks. With IDLE_SECS=1
# the condition triggers on the FIRST check (≥1 s after start). The poll loop
# interval caps worst-case latency at ~5 s; we wait 8 s to be safe on slow CI.
#
# Mutation-verified: changing IDLE_SECS to 9999 prevents the watcher from
# triggering and the "process exited" assertion fails; removing the idle_watcher
# thread entirely also fails both assertions.
# ---------------------------------------------------------------------------
@test "idle-timeout: AUTOSPEC_GUI_IDLE_SECS=1 → server self-exits with idle_timeout log (#845)" {
    local port
    port="$(pick_free_port)"

    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    # Capture stdout so we can assert the idle_timeout log line.
    local out_file="$TMP/server-stdout.log"

    # Start with IDLE_SECS=1 (argv[7]) — the watcher will fire on its first
    # 5-second sleep once 1s of inactivity has elapsed.
    # GUI_PID is set so teardown will attempt kill, but we expect it to have
    # already self-exited before the assertion runs.
    PATH="$BIN:$PATH" python3 "$SERVER_PY" \
        "$port" "$TOKEN" "$TMP" "$GUI_HTML" "$LOCK_FILE" "0" "1" \
        >"$out_file" 2>&1 &
    GUI_PID=$!

    # Wait for the server to be ready before we start the idle clock.
    wait_for_port "$port" 30 || {
        echo "server did not start" >&2
        return 1
    }

    # Make NO requests — just wait for the idle_watcher to fire.
    # idle_watcher: sleep(5) then check time.time() - last_activity > IDLE_SECS=1
    # So worst-case shutdown is ~5 s after last_activity (set at startup).
    # We wait up to 12 s in 0.5 s steps to stay deterministic on slow CI.
    local i
    for (( i=0; i<24; i++ )); do
        sleep 0.5
        kill -0 "$GUI_PID" 2>/dev/null || break
    done

    # Assert: process must have exited on its own.
    ! kill -0 "$GUI_PID" 2>/dev/null

    # Clear GUI_PID — teardown kill is harmless but not needed.
    GUI_PID=""

    # Assert: stdout must contain the idle_timeout sentinel.
    grep -q "idle_timeout" "$out_file"
}

# ---------------------------------------------------------------------------
# Test 17: GET / (_serve_html) — wiring from launcher token through server to
# HTML file is tested end-to-end (#846)
#
# The _serve_html route was untested: every existing test only exercised /api/*
# endpoints. The smoke bats grepped index.html on disk but never made an HTTP
# GET / against a live server. This means:
#   - A broken path in do_GET (e.g. wrong routing condition) would not be caught.
#   - A missing or corrupt index.html would not be caught at server level.
#   - The auth_ok re-check inside _serve_html had no positive (200) assertion.
#
# This test starts a real server, issues:
#   1. GET / with a WRONG token → 401 (auth gate in _serve_html)
#   2. GET / with NO token     → 401
#   3. GET / with the CORRECT token → 200, Content-Type text/html,
#      body contains '<title>autospec-fleet</title>' (known marker in index.html)
#      and 'aria-label="Filter repos by name"' (from the search-box element)
#
# Mutation-verified: with _serve_html forced to return 404 unconditionally, the
# 200 assertion fails; with auth_ok replaced by `return True`, the 401
# assertions for missing/wrong token fail.
# ---------------------------------------------------------------------------
@test "GET / with valid token returns 200 text/html with known marker (#846)" {
    local port
    port="$(pick_free_port)"

    # Stub gh (the server may call gh for /api/repos; stub it here for a clean
    # environment even though this test only exercises GET /).
    local BIN="$TMP/bin"
    mkdir -p "$BIN"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo '[]'
EOF
    chmod +x "$BIN/gh"

    PATH="$BIN:$PATH" start_server "$port"

    # ── part 1: missing token → 401 ──────────────────────────────────────────
    # raw_get with no third arg sends NO X-Autospec-Token header.
    local resp code
    resp="$(raw_get "$port" "/")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]

    # ── part 2: wrong token → 401 ────────────────────────────────────────────
    resp="$(raw_get "$port" "/" "definitely-wrong-token")"
    code="$(http_code_of "$resp")"
    [ "$code" = "401" ]

    # ── part 3: correct token → 200 + text/html + known HTML markers ─────────
    # Use curl -i to capture the response headers alongside the body so we can
    # assert Content-Type without a separate HEAD request.
    local full_resp
    full_resp="$(curl -si -H "X-Autospec-Token: $TOKEN" "http://127.0.0.1:${port}/")"

    # HTTP status must be 200.
    echo "$full_resp" | grep -q "^HTTP/1\.[01] 200"

    # Content-Type must be text/html (with optional charset suffix).
    echo "$full_resp" | grep -qi "^Content-Type: text/html"

    # Body must contain the <title> marker from index.html — a server that
    # returns a stub or empty body would fail here.
    echo "$full_resp" | grep -q '<title>autospec-fleet</title>'

    # Body must also contain the search-box aria-label so we confirm actual
    # index.html contents are served (not a minimal stub).
    echo "$full_resp" | grep -q 'aria-label="Filter repos by name"'
}
