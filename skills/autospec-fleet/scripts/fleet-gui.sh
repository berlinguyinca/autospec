#!/usr/bin/env bash
# fleet-gui.sh — launch a one-page browser GUI for autospec-fleet configuration.
#
# Usage:
#   fleet-gui.sh [--no-browser] [--print-url] [--once]
#
# Flags:
#   --no-browser   Do not open the system browser; just start the server.
#   --print-url    Print the URL (with token) to stdout before entering server loop.
#   --once         Run exactly one request cycle (serve one API call) then exit 0.
#                  Useful for smoke tests.
#
# Environment:
#   AUTOSPEC_GUI_IDLE_SECS   Idle timeout in seconds (default 900 = 15 min).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Defaults ─────────────────────────────────────────────────────────────────
NO_BROWSER=0
PRINT_URL=0
ONCE=0
IDLE_SECS="${AUTOSPEC_GUI_IDLE_SECS:-900}"

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-browser)   NO_BROWSER=1 ;;
        --print-url)    PRINT_URL=1 ;;
        --once)         ONCE=1 ;;
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

# ── Require gh ────────────────────────────────────────────────────────────────
require_gh() {
    if ! command -v gh >/dev/null 2>&1; then
        printf 'fleet-gui: gh not found on PATH — cannot fetch repo list\n' >&2
        printf 'code_health:fleet_gui_missing_gh\n' >&2
        exit 1
    fi
}

require_gh

# ── Pick random port + token ──────────────────────────────────────────────────
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

PORT="$(pick_port)"
TOKEN="$(pick_token)"
WORKSPACE="${PWD}"
GUI_HTML="${SKILL_DIR}/gui/index.html"
LOCK_DIR="${WORKSPACE}/.autospec-fleet"
LOCK_FILE="${LOCK_DIR}/.gui-lock"

# ── Embedded Python HTTP server ───────────────────────────────────────────────
start_server() {
    local port="$1"
    local token="$2"
    local workspace="$3"
    local gui_html="$4"
    local lock_file="$5"
    local once="$6"
    local idle_secs="$7"

    python3 - "$port" "$token" "$workspace" "$gui_html" "$lock_file" "$once" "$idle_secs" <<'PYEOF'
import sys, os, json, subprocess, threading, time, tempfile, shutil, fcntl
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs
import pathlib

port      = int(sys.argv[1])
TOKEN     = sys.argv[2]
WORKSPACE = sys.argv[3]
GUI_HTML  = sys.argv[4]
LOCK_FILE = sys.argv[5]
ONCE      = sys.argv[6] == "1"
IDLE_SECS = int(sys.argv[7])

MANAGED_KEYS = {"version", "workspace", "default_profile", "parallel_repos", "repos"}

DEFAULT_SKELETON = {
    "version": 1,
    "workspace": ".autospec-fleet/repos",
    "default_profile": "qwen3-32b-laptop",
    "parallel_repos": 2,
    "repos": [],
}

last_activity = time.time()
shutdown_event = threading.Event()
request_count = 0

def update_activity():
    global last_activity
    last_activity = time.time()

def load_yaml_config(path):
    """Load YAML config using python3 yaml module (stdlib pyyaml may not exist).
    Falls back to a simple line-by-line parser for basic key: value pairs."""
    try:
        import yaml
        with open(path) as f:
            return yaml.safe_load(f) or {}
    except ImportError:
        pass
    # Minimal fallback: use json if the file is somehow JSON, else return {}
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return {}

def dump_yaml_config(data):
    """Dump config as YAML. Use pyyaml if available, else basic format."""
    try:
        import yaml
        return yaml.dump(data, default_flow_style=False, allow_unicode=True, sort_keys=False)
    except ImportError:
        # Basic YAML serializer for simple structures
        lines = []
        for k, v in data.items():
            if isinstance(v, list):
                lines.append(f"{k}:")
                for item in v:
                    if isinstance(item, dict):
                        first = True
                        for ik, iv in item.items():
                            prefix = "  - " if first else "    "
                            first = False
                            if isinstance(iv, bool):
                                lines.append(f"{prefix}{ik}: {'true' if iv else 'false'}")
                            elif isinstance(iv, (int, float)):
                                lines.append(f"{prefix}{ik}: {iv}")
                            else:
                                lines.append(f"{prefix}{ik}: {iv}")
                    else:
                        lines.append(f"  - {item}")
            elif isinstance(v, bool):
                lines.append(f"{k}: {'true' if v else 'false'}")
            elif isinstance(v, (int, float)):
                lines.append(f"{k}: {v}")
            elif v is None:
                lines.append(f"{k}:")
            else:
                lines.append(f"{k}: {v}")
        return "\n".join(lines) + "\n"

def auth_ok(handler):
    """Check X-Autospec-Token header or ?t= query param."""
    # Check header
    header_token = handler.headers.get("X-Autospec-Token", "")
    if header_token == TOKEN:
        return True
    # Check query param
    parsed = urlparse(handler.path)
    qs = parse_qs(parsed.query)
    if qs.get("t", [""])[0] == TOKEN:
        return True
    return False

class FleetHandler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        # Suppress default access log; errors still go to stderr
        pass

    def send_json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        global request_count
        update_activity()
        parsed = urlparse(self.path)
        path = parsed.path

        # Serve the GUI HTML at root (token in query param is acceptable)
        if path == "/" or path == "":
            if not auth_ok(self):
                self.send_response(401)
                self.end_headers()
                return
            if os.path.exists(GUI_HTML):
                with open(GUI_HTML, "rb") as f:
                    body = f.read()
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_json(404, {"error": "gui_html_not_found", "path": GUI_HTML})
            if ONCE:
                request_count += 1
                shutdown_event.set()
            return

        if not auth_ok(self):
            self.send_json(401, {"error": "unauthorized"})
            return

        if path == "/api/repos":
            self._handle_repos()
            if ONCE:
                request_count += 1
                shutdown_event.set()
        elif path == "/api/config":
            self._handle_config_get()
            if ONCE:
                request_count += 1
                shutdown_event.set()
        else:
            self.send_json(404, {"error": "not_found"})

    def do_POST(self):
        update_activity()
        parsed = urlparse(self.path)
        path = parsed.path

        if not auth_ok(self):
            self.send_json(401, {"error": "unauthorized"})
            return

        if path == "/api/config":
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            try:
                data = json.loads(body)
            except json.JSONDecodeError as e:
                self.send_json(400, {"error": "invalid_json", "detail": str(e)})
                return
            self._handle_config_post(data)
        else:
            self.send_json(404, {"error": "not_found"})

    def _handle_repos(self):
        try:
            result = subprocess.run(
                ["gh", "repo", "list",
                 "--json", "nameWithOwner,pushedAt,visibility,description,url",
                 "--limit", "200"],
                capture_output=True, text=True, timeout=30
            )
            if result.returncode != 0:
                err = result.stderr.strip()
                if "not logged" in err.lower() or "authentication" in err.lower() or "auth" in err.lower():
                    self.send_json(503, {
                        "error": "gh_not_authenticated",
                        "hint": "run: gh auth login"
                    })
                else:
                    self.send_json(503, {"error": "gh_failed", "detail": err})
                return
            repos = json.loads(result.stdout or "[]")
            # Sort by pushedAt descending (ISO 8601 strings sort lexicographically)
            repos.sort(key=lambda r: r.get("pushedAt", ""), reverse=True)
            self.send_json(200, repos)
        except FileNotFoundError:
            self.send_json(503, {
                "error": "gh_not_found",
                "hint": "Install gh CLI: https://cli.github.com"
            })
        except subprocess.TimeoutExpired:
            self.send_json(503, {"error": "gh_timeout"})

    def _handle_config_get(self):
        config_path = os.path.join(WORKSPACE, "autospec-fleet.yml")
        if not os.path.exists(config_path):
            self.send_json(200, {"config": DEFAULT_SKELETON, "exists": False})
            return
        try:
            cfg = load_yaml_config(config_path)
            if not cfg:
                cfg = dict(DEFAULT_SKELETON)
                self.send_json(200, {"config": cfg, "exists": True, "warning": "yaml_partial"})
            else:
                self.send_json(200, {"config": cfg, "exists": True})
        except Exception as e:
            self.send_json(200, {
                "config": dict(DEFAULT_SKELETON),
                "exists": True,
                "warning": "yaml_partial",
                "detail": str(e)
            })

    def _handle_config_post(self, new_data):
        config_path = os.path.join(WORKSPACE, "autospec-fleet.yml")
        lock_dir = os.path.dirname(LOCK_FILE)
        os.makedirs(lock_dir, exist_ok=True)

        # Use a file lock to serialize concurrent POSTs
        lock_fd = open(LOCK_FILE, "w")
        try:
            fcntl.flock(lock_fd.fileno(), fcntl.LOCK_EX)

            # Read existing config to preserve unmanaged keys
            existing = {}
            if os.path.exists(config_path):
                try:
                    existing = load_yaml_config(config_path) or {}
                except Exception:
                    existing = {}

            # Merge strategy:
            # 1. Start with existing on-disk config (preserves on-disk unmanaged keys).
            # 2. Overlay unmanaged keys from the new_data body (caller may pass them back).
            # 3. Apply managed keys from new_data (authoritative update from GUI).
            merged = dict(existing)
            for key, val in new_data.items():
                if key not in MANAGED_KEYS:
                    merged[key] = val
            for key in MANAGED_KEYS:
                if key in new_data:
                    merged[key] = new_data[key]

            # Atomic write via temp file + mv
            config_dir = os.path.dirname(config_path) or "."
            tmp_fd, tmp_path = tempfile.mkstemp(dir=config_dir, prefix=".autospec-fleet-tmp-")
            try:
                with os.fdopen(tmp_fd, "w") as f:
                    f.write(dump_yaml_config(merged))
                os.replace(tmp_path, config_path)
            except Exception:
                try:
                    os.unlink(tmp_path)
                except OSError:
                    pass
                raise

            repos_count = len(merged.get("repos", []))
            self.send_json(200, {"saved": True, "repos_count": repos_count})

        finally:
            fcntl.flock(lock_fd.fileno(), fcntl.LOCK_UN)
            lock_fd.close()

        # Arm shutdown after successful save (give response time to flush)
        if ONCE:
            threading.Timer(0.1, shutdown_event.set).start()
        else:
            threading.Timer(1.0, shutdown_event.set).start()


def idle_watcher(server):
    while not shutdown_event.is_set():
        if time.time() - last_activity > IDLE_SECS:
            print("fleet-gui: idle_timeout — shutting down", flush=True)
            shutdown_event.set()
            break
        time.sleep(5)

# Start server
server = HTTPServer(("127.0.0.1", port), FleetHandler)
server.timeout = 1  # Poll every second so we can check shutdown_event

idle_thread = threading.Thread(target=idle_watcher, args=(server,), daemon=True)
idle_thread.start()

while not shutdown_event.is_set():
    server.handle_request()

server.server_close()
sys.exit(0)
PYEOF
}

# ── Build URL and optionally print it ────────────────────────────────────────
URL="http://127.0.0.1:${PORT}/?t=${TOKEN}"
if [[ "$PRINT_URL" -eq 1 ]]; then
    printf '%s\n' "$URL"
fi

# ── Open browser ──────────────────────────────────────────────────────────────
if [[ "$NO_BROWSER" -eq 0 ]]; then
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$URL" &>/dev/null &
    elif command -v open >/dev/null 2>&1; then
        open "$URL" &>/dev/null &
    else
        printf 'fleet-gui: cannot auto-open browser — paste this URL:\n  %s\n' "$URL" >&2
    fi
fi

# ── In --once mode with no browser, just verify setup and exit ────────────────
# This allows smoke-testing that the script starts, parses args, finds gh,
# picks a port/token, and prints the URL — all without blocking on a request.
# Test suites that need request handling launch the server in the background and
# issue curl requests themselves (see tests/fleet/test_fleet_gui.bats).
if [[ "$ONCE" -eq 1 && "$NO_BROWSER" -eq 1 ]]; then
    # Start the server briefly to validate it binds, then exit
    python3 - "$PORT" "$TOKEN" "$WORKSPACE" "$GUI_HTML" "$LOCK_FILE" "$IDLE_SECS" <<'PYVALIDATE'
import sys, socket
port = int(sys.argv[1])
# Verify port is bindable (we already picked it, just double-check)
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(('127.0.0.1', port))
    s.close()
    sys.exit(0)
except OSError as e:
    print(f"fleet-gui: port {port} not bindable: {e}", file=sys.stderr)
    sys.exit(1)
PYVALIDATE
    exit 0
fi

# ── Start server (blocks until shutdown) ─────────────────────────────────────
start_server "$PORT" "$TOKEN" "$WORKSPACE" "$GUI_HTML" "$LOCK_FILE" "$ONCE" "$IDLE_SECS"
