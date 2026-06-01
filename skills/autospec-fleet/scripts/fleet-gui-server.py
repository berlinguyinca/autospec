#!/usr/bin/env python3
"""fleet-gui-server.py — embedded HTTP server for the autospec-fleet GUI.

Called by fleet-gui.sh; not intended to be invoked directly.
Args: <port> <token> <workspace> <gui_html> <lock_file> <once> <idle_secs>
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


def record_activity(ts=None):
    """Record the current time as the last activity timestamp."""
    global last_activity
    last_activity = ts if ts is not None else time.time()


def _yaml_scalar(v):
    """Format a scalar value for basic YAML output."""
    if isinstance(v, bool):
        return "true" if v else "false"
    return str(v)


def _yaml_dict_items(item):
    """Yield YAML lines for a dict item in a list."""
    first = True
    for ik, iv in item.items():
        prefix = "  - " if first else "    "
        first = False
        yield f"{prefix}{ik}: {_yaml_scalar(iv)}"


def _yaml_list_items(lst):
    """Yield YAML lines for a list value."""
    for item in lst:
        if isinstance(item, dict):
            yield from _yaml_dict_items(item)
        else:
            yield f"  - {item}"


def _basic_yaml_dump(data):
    """Minimal YAML serializer for simple dict/list structures."""
    lines = []
    for k, v in data.items():
        if isinstance(v, list):
            lines.append(f"{k}:")
            lines.extend(_yaml_list_items(v))
        elif v is None:
            lines.append(f"{k}:")
        else:
            lines.append(f"{k}: {_yaml_scalar(v)}")
    return "\n".join(lines) + "\n"


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
        return yaml.dump(data, default_flow_style=False, allow_unicode=True, sort_keys=False)
    except ImportError:
        return _basic_yaml_dump(data)


def auth_ok(handler):
    """Return True if the request carries the correct URL token."""
    if handler.headers.get("X-Autospec-Token", "") == TOKEN:
        return True
    qs = parse_qs(urlparse(handler.path).query)
    return qs.get("t", [""])[0] == TOKEN


def schedule_shutdown(post_save=False):
    """Trigger server shutdown after a short delay (once-mode or post-save)."""
    delay = 1.0 if post_save and not ONCE else 0.1
    threading.Timer(delay, shutdown_event.set).start()


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
        record_activity(ts=None)
        path = urlparse(self.path).path
        if path in ("/", ""):
            self._serve_html()
        elif not auth_ok(self):
            self.send_json(401, {"error": "unauthorized"})
        elif path == "/api/repos":
            self._handle_repos()
            if ONCE:
                schedule_shutdown(post_save=False)
        elif path == "/api/config":
            self._handle_config_get()
            if ONCE:
                schedule_shutdown(post_save=False)
        else:
            self.send_json(404, {"error": "not_found"})

    def do_POST(self):
        record_activity(ts=None)
        path = urlparse(self.path).path
        if not auth_ok(self):
            self.send_json(401, {"error": "unauthorized"})
            return
        if path == "/api/config":
            self._read_and_handle_config_post()
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
            schedule_shutdown(post_save=False)

    def _handle_repos(self):
        try:
            result = subprocess.run(
                ["gh", "repo", "list",
                 "--json", "nameWithOwner,pushedAt,visibility,description,url",
                 "--limit", "200"],
                capture_output=True, text=True, timeout=30,
            )
        except FileNotFoundError:
            self.send_json(503, {"error": "gh_not_found", "hint": "Install gh CLI: https://cli.github.com"})
            return
        except subprocess.TimeoutExpired:
            self.send_json(503, {"error": "gh_timeout"})
            return
        if result.returncode != 0:
            self._repos_error(result.stderr.strip())
            return
        repos = json.loads(result.stdout or "[]")
        repos.sort(key=lambda r: r.get("pushedAt", ""), reverse=True)
        self.send_json(200, repos)

    def _repos_error(self, err):
        """Send appropriate error JSON for a failed gh repo list call."""
        hint_words = ("not logged", "authentication", "auth")
        if any(w in err.lower() for w in hint_words):
            self.send_json(503, {"error": "gh_not_authenticated", "hint": "run: gh auth login"})
        else:
            self.send_json(503, {"error": "gh_failed", "detail": err})

    def _handle_config_get(self):
        config_path = os.path.join(WORKSPACE, "autospec-fleet.yml")
        if not os.path.exists(config_path):
            self.send_json(200, {"config": DEFAULT_SKELETON, "exists": False})
            return
        try:
            cfg = load_yaml_config(config_path)
        except Exception as e:
            self.send_json(200, {"config": dict(DEFAULT_SKELETON), "exists": True, "warning": "yaml_partial", "detail": str(e)})
            return
        if not cfg:
            self.send_json(200, {"config": dict(DEFAULT_SKELETON), "exists": True, "warning": "yaml_partial"})
        else:
            self.send_json(200, {"config": cfg, "exists": True})

    def _read_and_handle_config_post(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        try:
            data = json.loads(body)
        except json.JSONDecodeError as e:
            self.send_json(400, {"error": "invalid_json", "detail": str(e)})
            return
        self._handle_config_post(data)

    def _handle_config_post(self, new_data):
        config_path = os.path.join(WORKSPACE, "autospec-fleet.yml")
        os.makedirs(os.path.dirname(LOCK_FILE), exist_ok=True)
        lock_fd = open(LOCK_FILE, "w")
        try:
            fcntl.flock(lock_fd.fileno(), fcntl.LOCK_EX)
            merged = _merge_config(config_path, new_data)
            _atomic_write(config_path, dump_yaml_config(merged))
            repos_count = len(merged.get("repos", []))
            self.send_json(200, {"saved": True, "repos_count": repos_count})
        finally:
            fcntl.flock(lock_fd.fileno(), fcntl.LOCK_UN)
            lock_fd.close()
        schedule_shutdown(post_save=True)


def _merge_config(config_path, new_data):
    """Merge new_data over existing on-disk config, preserving unmanaged keys."""
    existing = {}
    if os.path.exists(config_path):
        try:
            existing = load_yaml_config(config_path) or {}
        except Exception:
            existing = {}
    # Strategy: existing → unmanaged keys from new_data → managed keys from new_data
    merged = dict(existing)
    for key, val in new_data.items():
        if key not in MANAGED_KEYS:
            merged[key] = val
    for key in MANAGED_KEYS:
        if key in new_data:
            merged[key] = new_data[key]
    return merged


def _atomic_write(target_path, content):
    """Write content to target_path atomically via temp file + rename."""
    config_dir = os.path.dirname(os.path.abspath(target_path))
    tmp_fd, tmp_path = tempfile.mkstemp(dir=config_dir, prefix=".autospec-fleet-tmp-")
    try:
        with os.fdopen(tmp_fd, "w") as f:
            f.write(content)
        os.replace(tmp_path, target_path)
    except Exception:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


def idle_watcher():
    """Shut down the server after IDLE_SECS of no requests."""
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
