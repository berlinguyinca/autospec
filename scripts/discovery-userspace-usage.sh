#!/usr/bin/env bash
# discovery-userspace-usage.sh — mines the operator's own Claude Code session
# transcripts / tool-usage history for recurring friction (manual workarounds,
# failure-recovery loops) and appends DERIVED-ONLY trend signals via
# trend-ledger.sh. Mirrors mine-pr-history.sh's local-mining shape.
#
# PRIVACY (non-negotiable): raw transcript text is never written to the
# ledger. Every emitted record's sanitized_excerpt is a derived, normalized
# shape (command name + subcommand only — never the raw line/args) run
# through discovery-safety.sh --sanitize. When discovery.userspace.opt_out
# is true, this script exits 0 without reading anything — the config check
# happens before any history-dir access.
#
# Usage:
#   discovery-userspace-usage.sh <autospec.yml>
#
# Env:
#   AUTOSPEC_USERSPACE_HISTORY_DIR   session-history root (default: ~/.claude/projects)
#   AUTOSPEC_TREND_LEDGER            forwarded to trend-ledger.sh (ledger path)
#   AUTOSPEC_USERSPACE_MIN_RECURRENCE  in-scan occurrence threshold to qualify
#                                       as "recurring" (default: 2)
#
# Exit codes:
#   0  always — including opt-out, absent history dir, or no qualifying signals
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TREND_LEDGER="$SCRIPT_DIR/trend-ledger.sh"
DISCOVERY_SAFETY="$SCRIPT_DIR/discovery-safety.sh"

CFG="${1:?usage: discovery-userspace-usage.sh <autospec.yml>}"

# Read a config file (YAML or JSON) as JSON on stdout. yq handles both; jq is
# the JSON-only fallback when yq is absent. Mirrors discovery-safety.sh.
cfg_to_json() {
  local cfg="$1"
  if command -v yq >/dev/null 2>&1; then
    yq -o=json '.' "$cfg" 2>/dev/null
  else
    jq -e '.' "$cfg" 2>/dev/null
  fi
}

# ── Opt-out gate: MUST run before any history-dir read ───────────────────────
[ -f "$CFG" ] || exit 0

cfg_json="$(cfg_to_json "$CFG" || true)"
if [ -z "$cfg_json" ]; then
  exit 0
fi

opt_out="$(printf '%s' "$cfg_json" | jq -r '.discovery.userspace.opt_out // false' 2>/dev/null || echo false)"
if [ "$opt_out" = "true" ]; then
  exit 0
fi

# ── History scan ──────────────────────────────────────────────────────────────
HISTORY_DIR="${AUTOSPEC_USERSPACE_HISTORY_DIR:-$HOME/.claude/projects}"
MIN_RECURRENCE="${AUTOSPEC_USERSPACE_MIN_RECURRENCE:-2}"

[ -d "$HISTORY_DIR" ] || exit 0

command -v jq >/dev/null 2>&1 || exit 0

TMP_COMMANDS="$(mktemp)"
TMP_NORM="$(mktemp)"
cleanup() { rm -f "$TMP_COMMANDS" "$TMP_NORM"; }
trap cleanup EXIT

# Extract every Bash tool_use command from every *.jsonl transcript under the
# history dir. Best-effort: malformed lines/files are silently skipped (jq -c
# with -e off, redirected stderr) — this is read-only local mining, never a
# hard failure.
find "$HISTORY_DIR" -type f -name '*.jsonl' 2>/dev/null | while IFS= read -r f; do
  jq -r '
    select(.type=="assistant")
    | .message.content[]?
    | select(.type=="tool_use" and .name=="Bash")
    | .input.command // empty
  ' "$f" 2>/dev/null || true
done > "$TMP_COMMANDS"

[ -s "$TMP_COMMANDS" ] || exit 0

# Normalize each raw command line to a DERIVED shape: lowercase first two
# whitespace-separated tokens only (command + subcommand, e.g. "git status",
# "npm install"). Arguments, flags, paths, and any literal content are
# dropped entirely — this is the privacy boundary, not discovery-safety's
# --sanitize alone; the raw line never even reaches the excerpt builder.
while IFS= read -r cmd; do
  [ -z "$cmd" ] && continue
  norm="$(printf '%s' "$cmd" \
    | tr '[:upper:]' '[:lower:]' \
    | awk '{ if (NF >= 2) print $1, $2; else if (NF == 1) print $1 }' \
    | tr -cd 'a-z0-9 _.-')"
  [ -z "$norm" ] && continue
  printf '%s\n' "$norm"
done < "$TMP_COMMANDS" | sort | uniq -c | sort -rn > "$TMP_NORM"

[ -s "$TMP_NORM" ] || exit 0

now_ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# shellcheck disable=SC2034
while read -r count norm; do
  [ -z "$norm" ] && continue
  [ "$count" -lt "$MIN_RECURRENCE" ] && continue

  norm_key="userspace-usage:cmd:${norm// /_}"

  excerpt="$(printf 'recurring command pattern: %s' "$norm" | "$DISCOVERY_SAFETY" --sanitize)"
  summary="operator repeatedly ran commands matching pattern: ${norm}"

  existing_count="$("$TREND_LEDGER" --show --json 2>/dev/null \
    | jq --arg k "$norm_key" '[.[] | select(.norm_key==$k)] | length' 2>/dev/null || echo 0)"

  if [ "${existing_count:-0}" -gt 0 ]; then
    "$TREND_LEDGER" --bump-recurrence "$norm_key" >/dev/null 2>&1 || true
  else
    signal="$(jq -nc \
      --arg source "userspace-usage" \
      --arg kind "recurring-command" \
      --arg summary "$summary" \
      --arg norm_key "$norm_key" \
      --arg evidence_ref "userspace:session-history" \
      --arg first_seen "$now_ts" \
      --argjson recurrence "$count" \
      --arg sanitized_excerpt "$excerpt" \
      --arg ts "$now_ts" \
      '{source:$source, kind:$kind, summary:$summary, norm_key:$norm_key,
        evidence_ref:$evidence_ref, first_seen:$first_seen,
        recurrence:$recurrence, sanitized_excerpt:$sanitized_excerpt, ts:$ts}')"
    "$TREND_LEDGER" --append "$signal" >/dev/null 2>&1 || true
  fi
done < "$TMP_NORM"

exit 0
