#!/usr/bin/env bash
# advisor-report.sh — summarize advisor telemetry + evaluate the promotion gate.
#
# Reads the JSONL written by advisor-escalate.sh --phase record and prints a
# per-gate summary. When all four baseline flags are supplied, also emits a
# `promote` boolean = (observed-lgtm >= baseline-lgtm) AND (observed-cost <= baseline-cost).
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md §Telemetry
#
# Usage:
#   advisor-report.sh --telemetry <jsonl> [--json] \
#     [--baseline-lgtm <f> --observed-lgtm <f> --baseline-cost <n> --observed-cost <n>]
set -eu

TELEMETRY="" JSON_MODE=0
BL_LGTM="" OB_LGTM="" BL_COST="" OB_COST=""

while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --baseline-lgtm) BL_LGTM="${2:-}"; shift 2 ;;
    --observed-lgtm) OB_LGTM="${2:-}"; shift 2 ;;
    --baseline-cost) BL_COST="${2:-}"; shift 2 ;;
    --observed-cost) OB_COST="${2:-}"; shift 2 ;;
    --json) JSON_MODE=1; shift ;;
    --help|-h) printf 'Usage: advisor-report.sh --telemetry <jsonl> [gate flags]\n'; exit 0 ;;
    *) printf 'advisor-report.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

if [ -z "$TELEMETRY" ] || [ ! -f "$TELEMETRY" ]; then
  printf 'advisor-report.sh: telemetry file not found: %s\n' "$TELEMETRY" >&2
  exit 1
fi

# Validate any supplied baseline as numeric BEFORE handing it to jq --argjson
# (a non-numeric value would abort jq with a raw parse error under set -e).
for _pair in "baseline-lgtm:$BL_LGTM" "observed-lgtm:$OB_LGTM" \
             "baseline-cost:$BL_COST" "observed-cost:$OB_COST"; do
  _name="${_pair%%:*}"; _val="${_pair#*:}"
  if [ -n "$_val" ]; then
    case "$_val" in
      ''|*[!0-9.]*|*.*.*) printf 'advisor-report.sh: --%s must be numeric: %s\n' "$_name" "$_val" >&2; exit 1 ;;
    esac
  fi
done

# Per-gate aggregation. Read line-by-line with `fromjson?` so a single malformed
# telemetry line is skipped rather than aborting the whole report.
summary="$(jq -R 'fromjson? // empty' "$TELEMETRY" | jq -s '
  group_by(.gate) | map({
    key: .[0].gate,
    value: {
      calls: length,
      plan: (map(select(.verdict=="plan")) | length),
      correction: (map(select(.verdict=="correction")) | length),
      stop: (map(select(.verdict=="stop")) | length),
      over_budget: (map(select(.over_budget==true)) | length),
      total_tokens_out: (map(.tokens_out // 0) | add)
    }
  }) | from_entries
')"

# Promotion gate arithmetic (only when all four baselines are present).
promote_obj="{}"
if [ -n "$BL_LGTM" ] && [ -n "$OB_LGTM" ] && [ -n "$BL_COST" ] && [ -n "$OB_COST" ]; then
  promote="$(jq -n \
    --argjson bl "$BL_LGTM" --argjson ol "$OB_LGTM" \
    --argjson bc "$BL_COST" --argjson oc "$OB_COST" \
    '($ol >= $bl) and ($oc <= $bc)')"
  promote_obj="$(jq -n --argjson p "$promote" '{promote:$p}')"
fi

jq -n --argjson s "$summary" --argjson p "$promote_obj" '$s + $p'
