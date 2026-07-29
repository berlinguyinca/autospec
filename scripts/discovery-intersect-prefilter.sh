#!/usr/bin/env bash
# discovery-intersect-prefilter.sh — recurrence-threshold throttle for the
# discovery trend ledger, feeding Stage-2 explore intersect.
#
# Reads latest-per-norm_key trend signals via trend-ledger.sh --show --json,
# keeps only signals whose recurrence meets the threshold, sorts the
# survivors by recurrence descending, and emits a compact JSON array (jq -c)
# to stdout. No LLM here, no candidate emission — this is a deterministic
# prefilter only.
#
# Usage:
#   discovery-intersect-prefilter.sh [--min N]
#     --min N   recurrence threshold (default: ${AUTOSPEC_TREND_MIN_RECURRENCE:-2})
#
# Env:
#   AUTOSPEC_TREND_LEDGER          forwarded to trend-ledger.sh (ledger path)
#   AUTOSPEC_TREND_MIN_RECURRENCE  default threshold when --min is not given
#
# Exit codes:
#   0  always, on success — including an empty or absent ledger (emits [])
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TREND_LEDGER="$SCRIPT_DIR/trend-ledger.sh"

min="${AUTOSPEC_TREND_MIN_RECURRENCE:-2}"
while [ $# -gt 0 ]; do
  case "$1" in
    --min) min="${2:?--min requires a value}"; shift 2 ;;
    *) echo "usage: discovery-intersect-prefilter.sh [--min N]" >&2; exit 2 ;;
  esac
done

signals="$("$TREND_LEDGER" --show --json)"
if [ -z "$signals" ] || [ "$signals" = "null" ]; then
  echo '[]'
  exit 0
fi

echo "$signals" | jq -c --argjson n "$min" 'map(select(.recurrence >= $n)) | sort_by(-.recurrence)'
