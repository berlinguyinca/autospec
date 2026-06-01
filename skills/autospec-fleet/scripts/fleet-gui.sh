#!/usr/bin/env bash
# fleet-gui.sh — launch a one-page browser GUI for autospec-fleet configuration.
#
# Usage:
#   fleet-gui.sh [--no-browser] [--print-url] [--once]
#
# Flags:
#   --no-browser   Do not open the system browser; just start the server.
#   --print-url    Print the URL (with token) to stdout before entering server loop.
#   --once         Smoke-test mode: start the server, self-issue one GET to
#                  /api/repos and /api/config, assert both return 200, exit 0.
#
# Environment:
#   AUTOSPEC_GUI_IDLE_SECS   Idle timeout in seconds (default 900 = 15 min).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVER_PY="${SCRIPT_DIR}/fleet-gui-server.py"

NO_BROWSER=0
PRINT_URL=0
ONCE=0
IDLE_SECS="${AUTOSPEC_GUI_IDLE_SECS:-900}"

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --no-browser) NO_BROWSER=1 ;;
            --print-url)  PRINT_URL=1 ;;
            --once)       ONCE=1 ;;
            -h|--help)
                grep '^# ' "$0" | sed 's/^# //'
                exit 0
                ;;
            *)
                printf 'fleet-gui: unknown option: %s\n' "$1" >&2
                exit 2
                ;;
        esac
        shift
    done
}

require_gh() {
    if ! command -v gh >/dev/null 2>&1; then
        printf 'fleet-gui: gh not found on PATH — cannot fetch repo list\n' >&2
        printf 'code_health:fleet_gui_missing_gh\n' >&2
        exit 1
    fi
}

pick_port() {
    python3 -c "
import socket, random
for _ in range(20):
    p = random.randint(49152, 65535)
    try:
        s = socket.socket()
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(('127.0.0.1', p))
        s.close()
        print(p)
        break
    except OSError:
        pass
"
}

pick_token() {
    python3 -c "import secrets; print(secrets.token_hex(8))"
}

open_browser() {
    local url="$1"
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$url" &>/dev/null &
    elif command -v open >/dev/null 2>&1; then
        open "$url" &>/dev/null &
    else
        printf 'fleet-gui: cannot auto-open browser — paste this URL:\n  %s\n' "$url" >&2
    fi
}

# smoke_once <port> <token> <workspace> <gui_html> <lock_file>
# Launch the server in --once mode, poll until it is accepting connections
# (bounded readiness wait — no fixed sleeps), then self-issue one authenticated
# GET to /api/repos and /api/config. Print each endpoint's HTTP status, fail
# (exit 1) if either is not 200 or the server never came up, and exit 0 on
# success. The server self-shuts-down once both endpoints have been served; this
# function reaps it. Returns 0 only when the live HTTP surface answered both.
smoke_once() {
    local s_port="$1" s_token="$2" s_workspace="$3" s_gui_html="$4" s_lock="$5"

    python3 "$SERVER_PY" \
        "$s_port" "$s_token" "$s_workspace" "$s_gui_html" "$s_lock" "1" "$IDLE_SECS" &
    local server_pid=$!

    local rc=0
    python3 - "$s_port" "$s_token" "$server_pid" <<'PY' || rc=$?
import sys, time, socket
from urllib.request import urlopen, Request
from urllib.error import HTTPError, URLError

port = int(sys.argv[1])
token = sys.argv[2]

# Bounded readiness poll: wait up to ~10s for the server to accept connections.
deadline = time.time() + 10.0
ready = False
while time.time() < deadline:
    sk = socket.socket()
    sk.settimeout(0.3)
    try:
        sk.connect(("127.0.0.1", port))
        ready = True
        break
    except OSError:
        time.sleep(0.1)
    finally:
        sk.close()

if not ready:
    print("fleet-gui: smoke FAILED — server did not start", file=sys.stderr)
    sys.exit(1)

def hit(path):
    url = f"http://127.0.0.1:{port}{path}"
    req = Request(url, headers={"X-Autospec-Token": token})
    try:
        with urlopen(req, timeout=5) as resp:
            return resp.getcode()
    except HTTPError as e:
        return e.code
    except URLError as e:
        print(f"fleet-gui: smoke FAILED — {path}: {e}", file=sys.stderr)
        return 0

ok = True
for path in ("/api/repos", "/api/config"):
    code = hit(path)
    print(f"fleet-gui: smoke {path} -> {code}")
    if code != 200:
        ok = False

if not ok:
    print("fleet-gui: smoke FAILED — endpoint did not return 200", file=sys.stderr)
    sys.exit(1)

print("fleet-gui: smoke OK — /api/repos and /api/config both 200")
sys.exit(0)
PY

    # Reap the server (it self-shuts-down after serving both endpoints; bound the
    # wait so a hung server cannot block the smoke run).
    local waited=0
    while kill -0 "$server_pid" 2>/dev/null && [[ "$waited" -lt 50 ]]; do
        sleep 0.1
        waited=$((waited + 1))
    done
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true

    return "$rc"
}

main() {
    parse_args "$@"
    require_gh

    local port token workspace gui_html lock_file url
    port="$(pick_port)"
    token="$(pick_token)"
    workspace="${PWD}"
    gui_html="${SKILL_DIR}/gui/index.html"
    lock_file="${workspace}/.autospec-fleet/.gui-lock"
    url="http://127.0.0.1:${port}/?t=${token}"

    [[ "$PRINT_URL" -eq 1 ]] && printf '%s\n' "$url"
    [[ "$NO_BROWSER" -eq 0 ]] && open_browser "$url"

    # Smoke-test / --once mode: actually start the server, self-issue one GET to
    # /api/repos and /api/config, assert both return HTTP 200, then exit 0. This
    # is the documented end-to-end smoke guarantee (spec § Primary smoke test):
    # it proves the live HTTP surface answers, not merely that the port binds.
    # The server is launched with the ONCE flag (arg 6 = "1"); it self-shuts-down
    # once both endpoints have been served.
    if [[ "$ONCE" -eq 1 ]]; then
        smoke_once "$port" "$token" "$workspace" "$gui_html" "$lock_file"
        return
    fi

    exec python3 "$SERVER_PY" \
        "$port" "$token" "$workspace" "$gui_html" "$lock_file" "0" "$IDLE_SECS"
}

main "$@"
