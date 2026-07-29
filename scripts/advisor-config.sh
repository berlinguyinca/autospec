#!/usr/bin/env bash
# advisor-config.sh — resolve the advisor config for the current repo.
#
# Single declarative source: the `advisor:` block in .autospec/autospec.yml
# (override the path with AUTOSPEC_CONFIG_FILE). Env vars are CI/test overrides
# ONLY, never the primary interface. Precedence: env > yaml > built-in default.
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md §Configuration
#
# Usage:
#   advisor-config.sh --key policy                       # auto | on | off
#   advisor-config.sh --key budget.max_calls_per_issue   # integer
#   advisor-config.sh --key budget.guidance_char_cap     # integer
set -eu

KEY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --key) KEY="${2:-}"; shift 2 ;;
    --help|-h) printf 'Usage: advisor-config.sh --key <policy|budget.max_calls_per_issue|budget.guidance_char_cap>\n'; exit 0 ;;
    *) printf 'advisor-config.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

CONFIG_FILE="${AUTOSPEC_CONFIG_FILE:-.autospec/autospec.yml}"

# Read advisor.<dotted> from the YAML, printing empty on any miss/parse error.
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
node = d.get("advisor", {})
if not isinstance(node, dict):
    sys.exit(0)
for part in sys.argv[2].split("."):
    if isinstance(node, dict) and part in node:
        node = node[part]
    else:
        sys.exit(0)
# YAML 1.1 (PyYAML) parses bare on/off/yes/no as booleans. Normalize back to the
# on/off vocabulary so the documented `policy: off` kill switch is honored.
if node is True:
    print("on"); sys.exit(0)
if node is False:
    print("off"); sys.exit(0)
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
  policy)
    v="$(resolve "${AUTOSPEC_ADVISOR_POLICY:-}" "policy" "auto")"
    case "$v" in
      auto|on|off) printf '%s\n' "$v" ;;
      *) printf 'auto\n' ;;   # invalid value → safe default
    esac
    ;;
  budget.max_calls_per_issue)
    v="$(resolve "${AUTOSPEC_ADVISOR_MAX_USES:-}" "budget.max_calls_per_issue" "3")"
    case "$v" in ''|*[!0-9]*) v=3 ;; esac
    printf '%s\n' "$v"
    ;;
  budget.guidance_char_cap)
    v="$(resolve "${AUTOSPEC_ADVISOR_MAX_CHARS:-}" "budget.guidance_char_cap" "2800")"
    case "$v" in ''|*[!0-9]*) v=2800 ;; esac
    printf '%s\n' "$v"
    ;;
  *)
    printf 'advisor-config.sh: unknown key: %s\n' "$KEY" >&2
    exit 1
    ;;
esac
