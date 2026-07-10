#!/usr/bin/env bash
# grooming-config.sh — resolve the backlog-grooming config for the current repo.
#
# Single declarative source: the `grooming:` block in .autospec/autospec.yml
# (override the path with AUTOSPEC_CONFIG_FILE). Env vars are CI/test overrides
# ONLY, never the primary interface. Precedence: env > yaml > built-in default.
#
# Usage:
#   grooming-config.sh --key policy                            # auto | on | off
#   grooming-config.sh --key budget.max_issues_per_cycle       # integer
#   grooming-config.sh --key budget.groom_attempts_per_issue   # integer
set -eu

KEY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --key) KEY="${2:-}"; shift 2 ;;
    --help|-h) printf 'Usage: grooming-config.sh --key <policy|budget.max_issues_per_cycle|budget.groom_attempts_per_issue>\n'; exit 0 ;;
    *) printf 'grooming-config.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

CONFIG_FILE="${AUTOSPEC_CONFIG_FILE:-.autospec/autospec.yml}"

# Read grooming.<dotted> from the YAML, printing empty on any miss/parse error.
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
node = d.get("grooming", {})
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
    v="$(resolve "${AUTOSPEC_GROOMING_POLICY:-}" "policy" "auto")"
    case "$v" in
      auto|on|off) printf '%s\n' "$v" ;;
      *) printf 'auto\n' ;;   # invalid value → safe default
    esac
    ;;
  budget.max_issues_per_cycle)
    v="$(resolve "${AUTOSPEC_GROOMING_MAX_ISSUES:-}" "budget.max_issues_per_cycle" "5")"
    case "$v" in ''|*[!0-9]*) v=5 ;; esac
    printf '%s\n' "$v"
    ;;
  budget.groom_attempts_per_issue)
    v="$(resolve "${AUTOSPEC_GROOMING_GROOM_ATTEMPTS:-}" "budget.groom_attempts_per_issue" "2")"
    case "$v" in ''|*[!0-9]*) v=2 ;; esac
    printf '%s\n' "$v"
    ;;
  *)
    printf 'grooming-config.sh: unknown key: %s\n' "$KEY" >&2
    exit 1
    ;;
esac
