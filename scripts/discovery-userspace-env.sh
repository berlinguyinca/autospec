#!/usr/bin/env bash
# discovery-userspace-env.sh — userspace-env harvester: inventories installed
# CLIs (PATH), autospec skills, and MCP servers, diffs against what this repo
# already integrates, and appends integration-gap trend signals via
# trend-ledger.sh (source="userspace-env").
#
# Read-only inspection ONLY: a discovered binary is checked with `command -v`
# / `[ -x ]`, never executed. Untrusted environment data is derived into a
# short, sanitized excerpt (discovery-safety.sh --sanitize) before it ever
# reaches the ledger — it is DATA, not instructions, and only ever proposes a
# candidate for the downstream fail-closed verify stage.
#
# Usage:
#   discovery-userspace-env.sh <cfg>
#
# Config (<cfg>, YAML or JSON, §D discovery.* block):
#   discovery.userspace.opt_out   true  → emit nothing, exit 0 (checked first)
#
# Env overrides (fixture-driven testing; defaults probe the real machine):
#   DISCOVERY_ENV_PATH              colon-separated dirs to scan for CLIs
#                                    (default: $PATH)
#   DISCOVERY_ENV_SCAN_ROOT         repo root(s) grepped to decide whether a
#                                    tool/MCP server is already "integrated"
#                                    (default: this repo's skills/ + scripts/)
#   DISCOVERY_ENV_MCP_CONFIG        path to an MCP config JSON file
#                                    (default: first of $HOME/.claude.json,
#                                    $HOME/.claude/mcp.json that exists)
#   DISCOVERY_ENV_NOW               ISO-8601 timestamp override (tests)
#
# Exit codes:
#   0  always — best-effort harvester; missing cfg is the only hard failure (2)
set -euo pipefail

usage() { echo "usage: discovery-userspace-env.sh <cfg>" >&2; exit 2; }

CFG="${1:-}"
[ -n "$CFG" ] || usage
[ -f "$CFG" ] || { echo "discovery-userspace-env: config not found: $CFG" >&2; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SAFETY="$SCRIPT_DIR/discovery-safety.sh"
LEDGER="$SCRIPT_DIR/trend-ledger.sh"
SOURCE_NAME="userspace-env"

# ── config → JSON (mirrors discovery-safety.sh's cfg_to_json) ────────────────
cfg_to_json() {
  if command -v yq >/dev/null 2>&1; then
    yq -o=json '.' "$CFG" 2>/dev/null
  else
    jq -e '.' "$CFG" 2>/dev/null
  fi
}

CFG_JSON="$(cfg_to_json || true)"
if [ -z "$CFG_JSON" ]; then
  echo "discovery-userspace-env: cannot parse config: $CFG" >&2
  exit 2
fi

OPT_OUT="$(printf '%s' "$CFG_JSON" | jq -r '.discovery.userspace.opt_out // false')"
if [ "$OPT_OUT" = "true" ]; then
  exit 0
fi

# Fail closed on the shared rate limiter: if the source has hit its cap in the
# configured window, emit nothing this run rather than flood the ledger.
if ! "$SAFETY" --rate-ok "$SOURCE_NAME" "$CFG" >/dev/null 2>&1; then
  exit 0
fi

now_ts() {
  if [ -n "${DISCOVERY_ENV_NOW:-}" ]; then
    printf '%s' "$DISCOVERY_ENV_NOW"
  else
    date -u +%Y-%m-%dT%H:%M:%SZ
  fi
}

# ── "already integrated" check: build a single haystack file from the scan
# root ONCE, then do a literal (never regex) substring search per candidate
# name against that one file — O(tree) instead of O(tools × tree). Per the
# jq/grep metachar-injection lesson, matches are always literal strings.
SCAN_ROOT="${DISCOVERY_ENV_SCAN_ROOT:-}"
if [ -z "$SCAN_ROOT" ]; then
  SCAN_ROOT=""
  [ -d "$REPO_ROOT/skills" ] && SCAN_ROOT="$SCAN_ROOT $REPO_ROOT/skills"
  [ -d "$REPO_ROOT/scripts" ] && SCAN_ROOT="$SCAN_ROOT $REPO_ROOT/scripts"
fi

HAYSTACK="$(mktemp)"
trap 'rm -f "$HAYSTACK"' EXIT
if [ -n "$SCAN_ROOT" ]; then
  # shellcheck disable=SC2086
  find $SCAN_ROOT -type f \( -name '*.md' -o -name '*.sh' -o -name '*.json' -o -name '*.yml' -o -name '*.yaml' \) \
    -exec cat {} + > "$HAYSTACK" 2>/dev/null || true
fi

is_integrated() {
  # is_integrated <name> — true (0) if <name> appears literally anywhere in
  # the precomputed HAYSTACK; inspection only, never executes anything.
  local name="$1"
  [ -s "$HAYSTACK" ] || return 1
  grep -qF -- "$name" "$HAYSTACK" 2>/dev/null
}

# ── existing ledger norm_keys for this source (append vs bump-recurrence) ───
EXISTING_KEYS="$("$LEDGER" --show --json --source "$SOURCE_NAME" 2>/dev/null | jq -r '.[].norm_key' 2>/dev/null || true)"

# Per-run cap: bound both ledger growth and wall-clock time (each emit spawns
# trend-ledger.sh + the validator). A real machine's PATH can hold hundreds
# of gaps; only the top slice is useful per run — the rest resurface (and
# accumulate recurrence) on the next scheduled run.
MAX_SIGNALS="${DISCOVERY_ENV_MAX_SIGNALS:-25}"
SIGNAL_COUNT=0

already_seen() {
  local key="$1"
  printf '%s\n' "$EXISTING_KEYS" | grep -qxF -- "$key"
}

emit_signal() {
  # emit_signal <kind> <norm_key> <summary> <evidence_ref>
  [ "$SIGNAL_COUNT" -lt "$MAX_SIGNALS" ] || return 0
  local kind="$1" norm_key="$2" summary="$3" evidence_ref="$4"
  local excerpt
  excerpt="$(printf '%s' "$summary" | "$SAFETY" --sanitize)"
  local ts; ts="$(now_ts)"

  if already_seen "$norm_key"; then
    "$LEDGER" --bump-recurrence "$norm_key" >/dev/null
    SIGNAL_COUNT=$((SIGNAL_COUNT + 1))
    return 0
  fi

  local obj
  obj="$(jq -cn \
    --arg source "$SOURCE_NAME" \
    --arg kind "$kind" \
    --arg summary "$summary" \
    --arg norm_key "$norm_key" \
    --arg evidence_ref "$evidence_ref" \
    --arg first_seen "$ts" \
    --arg excerpt "$excerpt" \
    --arg ts "$ts" \
    '{source:$source, kind:$kind, summary:$summary, norm_key:$norm_key,
      evidence_ref:$evidence_ref, first_seen:$first_seen, recurrence:1,
      sanitized_excerpt:$excerpt, ts:$ts}')"
  "$LEDGER" --append "$obj" >/dev/null
  SIGNAL_COUNT=$((SIGNAL_COUNT + 1))
}

# ── 1. CLIs on PATH (inspection only — command -v / [ -x ], never executed) ─
scan_clis() {
  local pathvar="${DISCOVERY_ENV_PATH:-$PATH}"
  local dir tool seen=""
  local IFS=':'
  for dir in $pathvar; do
    [ "$SIGNAL_COUNT" -lt "$MAX_SIGNALS" ] || break
    [ -n "$dir" ] || continue
    [ -d "$dir" ] || continue
    local f
    for f in "$dir"/*; do
      [ "$SIGNAL_COUNT" -lt "$MAX_SIGNALS" ] || break
      [ -e "$f" ] || continue
      [ -f "$f" ] && [ -x "$f" ] || continue
      tool="$(basename "$f")"
      case " $seen " in *" $tool "*) continue ;; esac
      seen="$seen $tool"
      is_integrated "$tool" && continue
      emit_signal "cli-integration-gap" \
        "${SOURCE_NAME}:cli:${tool}" \
        "CLI '${tool}' is installed on PATH but not referenced by any autospec skill or script; evaluate for integration." \
        "$f"
    done
  done
}

# ── 2. autospec skills inventory (enumerated; feeds the integrated-corpus
# scan above — no separate signal, a locally-owned skill is by definition
# integrated). ────────────────────────────────────────────────────────────
scan_skills() {
  local skills_dir="${DISCOVERY_ENV_SKILLS_DIR:-$REPO_ROOT/skills}"
  [ -d "$skills_dir" ] || return 0
  local d
  for d in "$skills_dir"/*/; do
    [ -d "$d" ] || continue
    : # enumerated for completeness; membership already covered by is_integrated's scan
  done
}

# ── 3. MCP servers configured in the environment ────────────────────────────
scan_mcp() {
  local cfg="${DISCOVERY_ENV_MCP_CONFIG:-}"
  if [ -z "$cfg" ]; then
    if [ -f "$HOME/.claude.json" ]; then
      cfg="$HOME/.claude.json"
    elif [ -f "$HOME/.claude/mcp.json" ]; then
      cfg="$HOME/.claude/mcp.json"
    fi
  fi
  [ -n "$cfg" ] && [ -f "$cfg" ] || return 0

  local names
  names="$(jq -r '.mcpServers // {} | keys[]?' "$cfg" 2>/dev/null || true)"
  [ -n "$names" ] || return 0

  local name
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    is_integrated "$name" && continue
    emit_signal "mcp-integration-gap" \
      "${SOURCE_NAME}:mcp:${name}" \
      "MCP server '${name}' is configured but not referenced by any autospec skill or script; evaluate for integration." \
      "$cfg"
  done <<EOF
$names
EOF
}

scan_skills
scan_clis
scan_mcp

exit 0
