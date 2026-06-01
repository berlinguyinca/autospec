#!/usr/bin/env python3
"""fleet-gui-server.py — embedded HTTP server for the autospec-fleet GUI.

Called by fleet-gui.sh; not intended to be invoked directly.

Usage: fleet-gui-server.py <port> <token> <workspace> <gui_html> <lock_file>
                           <once> <idle_secs>
"""

import sys
import os
import json
import subprocess
import threading
import time
import tempfile
import fcntl
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

port = int(sys.argv[1])
TOKEN = sys.argv[2]
WORKSPACE = sys.argv[3]
GUI_HTML = sys.argv[4]
LOCK_FILE = sys.argv[5]
ONCE = sys.argv[6] == "1"
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


def update_activity():
    global last_activity
    last_activity = time.time()


def load_yaml_config(path):
    """Load YAML config. Uses PyYAML if available; falls back to JSON."""
    try:
        import yaml
        with open(path) as f:
            return yaml.safe_load(f) or {}
    except ImportError:
        pass
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return {}


def dump_yaml_config(data):
    """Dump config dict as YAML. Uses PyYAML if available; falls back to basic."""
    try:
        import yaml
        return yaml.dump(data, default_flow_style=False, allow_unicode=True,
                         sort_keys=False)
    except ImportError:
        return _basic_yaml_dump(data)


def _basic_yaml_dump(data):
    """Minimal YAML serializer for simple dict/list structures."""
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
                        lines.append(f"{prefix}{ik}: {_yaml_scalar(iv)}")
                else:
                    lines.append(f"  - {item}")
        elif v is None:
            lines.append(f"{k}:")
        else:
            lines.append(f"{k}: {_yaml_scalar(v)}")
    return "\n".join(lines) + "\n"


def _yaml_scalar(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    return str(v)


def auth_ok(handler):
    """Return True if the request carries the correct URL token."""
    if handler.headers.get("X-Autospec-Token", "") == TOKEN:
        return True
    qs = parse_qs(urlparse(handler.path).query)
    return qs.get("t", [""])[0] == TOKEN


class FleetHandler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass  # suppress default access log

    def send_json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        update_activity()
        path = urlparse(self.path).path
        if path in ("/", ""):
            self._serve_html()
        elif not auth_ok(self):
            self.send_json(401, {"error": "unauthorized"})
        elif path == "/api/repos":
            self._handle_repos()
            if ONCE:
                threading.Timer(0.1, shutdown_event.set).start()
        elif path == "/api/config":
            self._handle_config_get()
            if ONCE:
                threading.Timer(0.1, shutdown_event.set).start()
        else:
            self.send_json(404, {"error": "not_found"})

    def do_POST(self):
        update_activity()
        path = urlparse(self.path).path
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

    def _serve_html(self):
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
            threading.Timer(0.1, shutdown_event.set).start()

    def _handle_repos(self):
        try:
            result = subprocess.run(
                ["gh", "repo", "list",
                 "--json", "nameWithOwner,pushedAt,visibility,description,url",
                 "--limit", "200"],
                capture_output=True, text=True, timeout=30,
            )
            if result.returncode != 0:
                err = result.stderr.strip()
                hint_words = ("not logged", "authentication", "auth")
                if any(w in err.lower() for w in hint_words):
                    self.send_json(503, {
                        "error": "gh_not_authenticated",
                        "hint": "run: gh auth login",
                    })
                else:
                    self.send_json(503, {"error": "gh_failed", "detail": err})
                return
            repos = json.loads(result.stdout or "[]")
            repos.sort(key=lambda r: r.get("pushedAt", ""), reverse=True)
            self.send_json(200, repos)
        except FileNotFoundError:
            self.send_json(503, {
                "error": "gh_not_found",
                "hint": "Install gh CLI: https://cli.github.com",
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
                self.send_json(200, {
                    "config": dict(DEFAULT_SKELETON),
                    "exists": True,
                    "warning": "yaml_partial",
                })
            else:
                self.send_json(200, {"config": cfg, "exists": True})
        except Exception as e:
            self.send_json(200, {
                "config": dict(DEFAULT_SKELETON),
                "exists": True,
                "warning": "yaml_partial",
                "detail": str(e),
            })

    def _handle_config_post(self, new_data):
        config_path = os.path.join(WORKSPACE, "autospec-fleet.yml")
        os.makedirs(os.path.dirname(LOCK_FILE), exist_ok=True)
        lock_fd = open(LOCK_FILE, "w")
        try:
            fcntl.flock(lock_fd.fileno(), fcntl.LOCK_EX)
            existing = {}
            if os.path.exists(config_path):
                try:
                    existing = load_yaml_config(config_path) or {}
                except Exception:
                    existing = {}
            # Merge: existing → unmanaged keys from new_data → managed keys from new_data
            merged = dict(existing)
            for key, val in new_data.items():
                if key not in MANAGED_KEYS:
                    merged[key] = val
            for key in MANAGED_KEYS:
                if key in new_data:
                    merged[key] = new_data[key]
            # Atomic write
            config_dir = os.path.dirname(os.path.abspath(config_path))
            tmp_fd, tmp_path = tempfile.mkstemp(
                dir=config_dir, prefix=".autospec-fleet-tmp-"
            )
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
        delay = 0.1 if ONCE else 1.0
        threading.Timer(delay, shutdown_event.set).start()


def idle_watcher():
    while not shutdown_event.is_set():
        if time.time() - last_activity > IDLE_SECS:
            print("fleet-gui: idle_timeout — shutting down", flush=True)
            shutdown_event.set()
            break
        time.sleep(5)


server = HTTPServer(("127.0.0.1", port), FleetHandler)
server.timeout = 1

idle_thread = threading.Thread(target=idle_watcher, daemon=True)
idle_thread.start()

while not shutdown_event.is_set():
    server.handle_request()

server.server_close()
sys.exit(0)
