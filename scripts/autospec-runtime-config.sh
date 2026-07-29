#!/usr/bin/env bash
# autospec-runtime-config.sh — shared runtime config resolver for autonomous knobs.

autospec_runtime_script_dir() {
    cd "$(dirname "${BASH_SOURCE[0]}")" && pwd
}

autospec_runtime_config_file() {
    if [ -n "${AUTOSPEC_CONFIG_FILE:-}" ]; then
        printf '%s\n' "$AUTOSPEC_CONFIG_FILE"
        return 0
    fi
    if [ -f ".autospec/autospec.yml" ]; then
        printf '%s\n' ".autospec/autospec.yml"
        return 0
    fi
    local _runtime_dir
    _runtime_dir="$(autospec_runtime_script_dir)"
    if [ -f "$_runtime_dir/../.autospec/autospec.yml" ]; then
        printf '%s\n' "$_runtime_dir/../.autospec/autospec.yml"
        return 0
    fi
    printf '%s\n' ".autospec/autospec.yml"
}

autospec_runtime_config_get() {
    local _key="$1"
    local _default="${2:-}"
    local _file
    _file="$(autospec_runtime_config_file)"
    if [ ! -f "$_file" ] || ! command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "$_default"
        return 0
    fi
    python3 - "$_file" "$_key" "$_default" <<'PY'
import json
import sys

path, dotted_key, default = sys.argv[1], sys.argv[2], sys.argv[3]

def parse_scalar(value):
    value = value.strip()
    if not value:
        return ""
    if value in {"true", "false"}:
        return value == "true"
    if (value.startswith("'") and value.endswith("'")) or (value.startswith('"') and value.endswith('"')):
        return value[1:-1]
    try:
        return int(value)
    except ValueError:
        return value

def simple_yaml_load(text):
    root = {}
    stack = [(-1, root)]
    for raw in text.splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        stripped = raw.strip()
        if stripped.startswith("- "):
            continue
        if ":" not in stripped:
            continue
        indent = len(raw) - len(raw.lstrip(" "))
        key, value = stripped.split(":", 1)
        key = key.strip()
        while stack and indent <= stack[-1][0]:
            stack.pop()
        parent = stack[-1][1]
        if value.strip() == "":
            child = {}
            parent[key] = child
            stack.append((indent, child))
        else:
            parent[key] = parse_scalar(value)
    return root

try:
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
except OSError:
    print(default)
    sys.exit(0)

data = None
try:
    import yaml  # type: ignore
    data = yaml.safe_load(text) or {}
except Exception:
    data = simple_yaml_load(text)

current = data
for part in dotted_key.split("."):
    if isinstance(current, dict) and part in current:
        current = current[part]
    else:
        print(default)
        sys.exit(0)

if isinstance(current, bool):
    print("true" if current else "false")
elif isinstance(current, (dict, list)):
    print(json.dumps(current, separators=(",", ":")))
else:
    print(current)
PY
}

autospec_runtime_cpu_count() {
    local _count=""
    _count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
    case "$_count" in *[!0-9]*|'') _count="" ;; esac
    if [ -z "$_count" ]; then
        _count="$(sysctl -n hw.ncpu 2>/dev/null || true)"
        case "$_count" in *[!0-9]*|'') _count="" ;; esac
    fi
    if [ -z "$_count" ]; then
        _count="$(nproc 2>/dev/null || true)"
        case "$_count" in *[!0-9]*|'') _count="" ;; esac
    fi
    [ -n "$_count" ] || _count=1
    printf '%s\n' "$_count"
}

autospec_runtime_discover_repo_workers() {
    local _cpu _workers
    _cpu="$(autospec_runtime_cpu_count)"
    _workers=$(( _cpu / 4 ))
    [ "$_workers" -ge 1 ] || _workers=1
    [ "$_workers" -le 4 ] || _workers=4
    printf '%s\n' "$_workers"
}

autospec_runtime_int_value() {
    local _value="$1"
    local _default="$2"
    case "$_value" in *[!0-9]*|'') printf '%s\n' "$_default" ;; *) printf '%s\n' "$_value" ;; esac
}

autospec_runtime_config_int() {
    local _key="$1"
    local _env_name="$2"
    local _default="$3"
    local _value
    _value="$(autospec_runtime_config_get "$_key" "")"
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" "$_default"
        return 0
    fi
    eval "_value=\${${_env_name}:-}"
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" "$_default"
        return 0
    fi
    printf '%s\n' "$_default"
}

autospec_runtime_config_path() {
    local _key="$1"
    local _env_name="$2"
    local _default="$3"
    local _value
    _value="$(autospec_runtime_config_get "$_key" "")"
    if [ -n "$_value" ] && [ "$_value" != "auto" ]; then
        printf '%s\n' "$_value"
        return 0
    fi
    eval "_value=\${${_env_name}:-}"
    if [ -n "$_value" ]; then
        printf '%s\n' "$_value"
        return 0
    fi
    printf '%s\n' "$_default"
}

autospec_runtime_repo_workers() {
    local _value
    _value="$(autospec_runtime_config_get "autonomous.concurrency.max_concurrent_repo_workers" "")"
    if [ "$_value" = "auto" ]; then
        autospec_runtime_discover_repo_workers
        return 0
    fi
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" 0
        return 0
    fi
    _value="${AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS:-}"
    if [ "$_value" = "auto" ]; then
        autospec_runtime_discover_repo_workers
        return 0
    fi
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" 0
        return 0
    fi
    autospec_runtime_discover_repo_workers
}

autospec_runtime_batch_size() {
    local _value
    _value="$(autospec_runtime_config_get "autonomous.concurrency.batch_size" "")"
    if [ "$_value" = "auto" ]; then
        autospec_runtime_repo_workers
        return 0
    fi
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" 1
        return 0
    fi
    _value="${AUTOSPEC_BATCH_SIZE:-}"
    if [ "$_value" = "auto" ]; then
        autospec_runtime_repo_workers
        return 0
    fi
    if [ -n "$_value" ]; then
        autospec_runtime_int_value "$_value" 1
        return 0
    fi
    printf '%s\n' 1
}
