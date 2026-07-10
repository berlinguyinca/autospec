#!/usr/bin/env bash
# growth-content-quality-precheck.sh — deterministic pre-checks feeding the
# content-quality gate: keyword-density ceiling + citation presence. Delegates
# FTC disclosure to growth-ethics-precheck.sh. Fail-closed.
set -euo pipefail

content="${1:-}"
[ -n "$content" ] || { echo "usage: growth-content-quality-precheck.sh <content.md>" >&2; exit 2; }
[ -f "$content" ] || { echo "content not found: $content" >&2; exit 2; }

KW_MAX="${GROWTH_KW_DENSITY_MAX:-0.06}"
MIN_CITATIONS="${GROWTH_MIN_CITATIONS:-0}"

# --- keyword density: max single-token share over total tokens ---------------
# Lowercase, split on non-alphanumeric, drop empties, count most frequent / total.
density="$(tr '[:upper:]' '[:lower:]' < "$content" \
  | tr -c 'a-z0-9' '\n' \
  | grep -v '^$' \
  | awk '{c[$0]++; n++} END { if (n==0){print 0; exit} m=0; for (k in c) if (c[k]>m) m=c[k]; printf "%.6f", m/n }')"

# Compare density > KW_MAX using awk (no bc dependency).
if awk -v d="$density" -v m="$KW_MAX" 'BEGIN{ exit !(d > m) }'; then
  echo "content-quality: keyword density $density exceeds max $KW_MAX (stuffing)" >&2
  exit 1
fi

# --- citation presence -------------------------------------------------------
if [ "$MIN_CITATIONS" -gt 0 ]; then
  links="$(grep -Eoc 'https?://[^[:space:])]+' "$content" || true)"
  [ -n "$links" ] || links=0
  if [ "$links" -lt "$MIN_CITATIONS" ]; then
    echo "content-quality: $links citation(s) found, need >= $MIN_CITATIONS" >&2
    exit 1
  fi
fi

# --- disclosure (delegate; skip if the Plan 1 script is unavailable) ---------
here="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$here/growth-ethics-precheck.sh" ]; then
  bash "$here/growth-ethics-precheck.sh" --disclosure "$content"
else
  echo "content-quality: growth-ethics-precheck.sh not found, skipping disclosure check" >&2
fi

exit 0
