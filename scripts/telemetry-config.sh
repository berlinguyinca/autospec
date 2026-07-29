#!/usr/bin/env bash
# telemetry-config.sh — resolve the autospec-db telemetry config for the
# current repo.
#
# Single declarative source: the `telemetry:` block in .autospec/autospec.yml
# (override the path with AUTOSPEC_CONFIG_FILE). Env vars are CI/test overrides
# ONLY, never the primary interface. Precedence: env > yaml > built-in default.
# Mirrors skills/autospec-shared/scripts/advisor-config.sh.
#
# Spec: docs/specs/2026-07-10-autospec-db-telemetry-design.md §Configuration
# Contract: docs/contracts/autospec-events-v1.md §Configuration surface
#
# Usage:
#   telemetry-config.sh --key enabled                # true | false
#   telemetry-config.sh --key host_label              # string ('' = short hostname)
#   telemetry-config.sh --key spool_max_bytes          # integer
#   telemetry-config.sh --key install.db_module        # prompt | always | never
set -eu

KEY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --key) KEY="${2:-}"; shift 2 ;;
    --help|-h) printf 'Usage: telemetry-config.sh --key <enabled|host_label|spool_max_bytes|install.db_module>\n'; exit 0 ;;
    *) printf 'telemetry-config.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

CONFIG_FILE="${AUTOSPEC_CONFIG_FILE:-.autospec/autospec.yml}"

# Read telemetry.<dotted> from the YAML, printing empty on any miss/parse error.
yaml_get() {
  local path="$1"
  [ -f "$CONFIG_FILE" ] || return 0
  command -v python3 >/dev/null 2>&1 || return 0
  python3 - "$CONFIG_FILE" "$path" <<'PY' 2>/dev/null || true
import sys, yaml
try:
    d = yaml.safe_load(open(sys.argv[1])) or {}
except Exception:
    sys.exit(0)
node = d.get("telemetry", {})
if not isinstance(node, dict):
    sys.exit(0)
for part in sys.argv[2].split("."):
    if isinstance(node, dict) and part in node:
        node = node[part]
    else:
        sys.exit(0)
# YAML 1.1 (PyYAML) parses bare true/false as booleans. Normalize back to the
# true/false vocabulary so callers can do a plain string comparison.
if node is True:
    print("true"); sys.exit(0)
if node is False:
    print("false"); sys.exit(0)
if isinstance(node, (dict, list)):
    sys.exit(0)
print(node)
PY
}

resolve() {
  local env_override="$1" yaml_path="$2" default="$3" val=""
  # 1. env override (test/CI hatch)
  if [ -n "$env_override" ]; then
    printf '%s' "$env_override"
    return 0
  fi
  # 2. yaml
  val="$(yaml_get "$yaml_path")"
  if [ -n "$val" ]; then
    printf '%s' "$val"
    return 0
  fi
  # 3. default
  printf '%s' "$default"
}

case "$KEY" in
  enabled)
    v="$(resolve "${AUTOSPEC_TELEMETRY_ENABLED:-}" "enabled" "true")"
    case "$v" in
      true|false) printf '%s\n' "$v" ;;
      *) printf 'true\n' ;;   # invalid value → safe default (enabled)
    esac
    ;;
  host_label)
    printf '%s\n' "$(resolve "${AUTOSPEC_DB_HOST_LABEL:-}" "host_label" "")"
    ;;
  spool_max_bytes)
    v="$(resolve "${AUTOSPEC_DB_SPOOL_MAX_BYTES:-}" "spool_max_bytes" "10485760")"
    case "$v" in ''|*[!0-9]*) v=10485760 ;; esac
    printf '%s\n' "$v"
    ;;
  install.db_module)
    v="$(resolve "${AUTOSPEC_INSTALL_DB_MODULE:-}" "install.db_module" "prompt")"
    case "$v" in
      prompt|always|never) printf '%s\n' "$v" ;;
      *) printf 'prompt\n' ;;   # invalid value → safe default
    esac
    ;;
  *)
    printf 'telemetry-config.sh: unknown key: %s\n' "$KEY" >&2
    exit 1
    ;;
esac
