#!/usr/bin/env bash
# growth-attribute.sh — deterministic v1 attribution: split the total positive
# metric delta evenly across lenses that shipped in the window. Fail-closed.
set -euo pipefail

before="${1:-}"; after="${2:-}"; ledger="${3:-}"
[ -n "$before" ] && [ -n "$after" ] && [ -n "$ledger" ] || {
  echo "usage: growth-attribute.sh <before.json> <after.json> <ledger.jsonl>" >&2; exit 2; }
for f in "$before" "$after" "$ledger"; do
  [ -f "$f" ] || { echo "not found: $f" >&2; exit 2; }
done
jq . "$before" >/dev/null 2>&1 || { echo "malformed before: $before" >&2; exit 1; }
jq . "$after"  >/dev/null 2>&1 || { echo "malformed after: $after" >&2; exit 1; }
# Ledger is JSONL; validate each line.
if [ -s "$ledger" ] && ! jq -e . "$ledger" >/dev/null 2>&1; then
  echo "malformed ledger (fail-closed): $ledger" >&2; exit 1
fi

# Total positive delta = sum over shared metric keys of max(after-before, 0).
delta="$(jq -n --slurpfile b "$before" --slurpfile a "$after" '
  ($b[0].metrics // {}) as $bm | ($a[0].metrics // {}) as $am
  | [ $am | to_entries[] | select((.value|type)=="number")
      | (.value - (($bm[.key]) // 0)) | select(. > 0) ] | add // 0')"

# Lenses that shipped (merged_clean|published), unique sources.
jq -s --argjson delta "$delta" '
  [ .[] | select(.outcome=="merged_clean" or .outcome=="published") | .source ]
  | unique as $sources
  | ($sources | length) as $n
  | if $n == 0 then []
    else [ $sources[] | { source: ., shipped: 1, delta_share: ($delta / $n) } ]
    end' "$ledger"
