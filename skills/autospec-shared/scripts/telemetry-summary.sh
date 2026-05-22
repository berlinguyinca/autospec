#!/usr/bin/env bash
# skills/autospec-shared/scripts/telemetry-summary.sh — Summarise cache-hit rate + token usage.
#
# Usage:
#   telemetry-summary.sh [--since <iso-datetime>] [--role <role>]
#
# Reads $AUTOSPEC_TELEMETRY_FILE (default: ~/.autospec/telemetry.jsonl) and emits:
#   - Total dispatches
#   - Overall cache-hit rate (rows where cache_read_input_tokens > 0)
#   - Per-role breakdown: dispatches, hits, hit-rate, total tokens
#
# Environment overrides (for testing):
#   AUTOSPEC_TELEMETRY_FILE — path to telemetry JSONL
#
# Exit codes:
#   0  Success (including empty file)
#   1  File not found or argument error
#
# Compatible with bash 3.2+

set -eu

SINCE=""
ROLE_FILTER=""

while [ $# -gt 0 ]; do
  case "$1" in
    --since)
      if [ $# -lt 2 ]; then printf 'telemetry-summary.sh: --since requires an argument\n' >&2; exit 1; fi
      SINCE="$2"; shift 2 ;;
    --role)
      if [ $# -lt 2 ]; then printf 'telemetry-summary.sh: --role requires an argument\n' >&2; exit 1; fi
      ROLE_FILTER="$2"; shift 2 ;;
    -*)
      printf 'telemetry-summary.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
    *)
      printf 'telemetry-summary.sh: unexpected argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

TELEMETRY_FILE="${AUTOSPEC_TELEMETRY_FILE:-$HOME/.autospec/telemetry.jsonl}"

if [ ! -f "$TELEMETRY_FILE" ]; then
  printf 'telemetry-summary.sh: telemetry file not found: %s\n' "$TELEMETRY_FILE" >&2
  exit 1
fi

jq -rs --arg since "$SINCE" --arg role "$ROLE_FILTER" '
  (. // []) as $all_rows
  | ($all_rows
      | map(
          if $since != "" then select(.ts >= $since) else . end
        )
      | map(
          if $role  != "" then select(.role == $role) else . end
        )
    ) as $rows
  | ($rows | length) as $total
  | ($rows | map(select(.cache_read_input_tokens > 0)) | length) as $hits
  | (if $total > 0 then ($hits * 100.0 / $total) else 0 end) as $hit_rate
  | ($rows | map(.input_tokens // 0) | add // 0) as $total_input
  | ($rows | map(.output_tokens // 0) | add // 0) as $total_output
  | ($rows | map(.cache_read_input_tokens // 0) | add // 0) as $total_cache_read
  | ($rows | group_by(.role) | map({
      role: .[0].role,
      dispatches: length,
      hits: (map(select(.cache_read_input_tokens > 0)) | length),
      hit_rate: (if length > 0 then ((map(select(.cache_read_input_tokens > 0)) | length) * 100.0 / length) else 0 end),
      total_tokens: (map((.input_tokens // 0) + (.output_tokens // 0)) | add // 0)
    })) as $by_role
  | "=== Autospec Telemetry Summary ===",
    "Total dispatches : \($total)",
    "Cache hits        : \($hits)",
    (if $total > 0
     then "Overall hit rate  : \($hit_rate | . * 10 | round / 10)%"
     else "Overall hit rate  : 0%"
     end),
    "Total input tokens: \($total_input)",
    "Total output tokens: \($total_output)",
    "Total cache reads : \($total_cache_read)",
    "",
    "Per-role breakdown:",
    (if ($by_role | length) > 0
     then ($by_role[] | "  \(.role): \(.dispatches) dispatch(es), \(.hits) hit(s), \(.hit_rate | . * 10 | round / 10)% hit rate, \(.total_tokens) total tokens")
     else "  (no data)"
     end)
' "$TELEMETRY_FILE"
