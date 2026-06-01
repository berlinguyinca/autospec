#!/usr/bin/env bash
# fleet-gui.sh — launch a one-page browser GUI for autospec-fleet configuration.
#
# Usage:
#   fleet-gui.sh [--no-browser] [--print-url] [--once]
#
# Flags:
#   --no-browser   Do not open the system browser; just start the server.
#   --print-url    Print the URL (with token) to stdout before entering server loop.
#   --once         Smoke-test mode: verify setup and exit 0 without serving.
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

    # Smoke-test / --once mode: verify bindable port then exit.
    if [[ "$ONCE" -eq 1 ]]; then
        python3 -c "
import sys, socket
p = int('${port}')
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(('127.0.0.1', p))
    s.close()
    sys.exit(0)
except OSError as e:
    print(f'fleet-gui: port {p} not bindable: {e}', file=sys.stderr)
    sys.exit(1)
"
        return
    fi

    exec python3 "$SERVER_PY" \
        "$port" "$token" "$workspace" "$gui_html" "$lock_file" "0" "$IDLE_SECS"
}

main "$@"
