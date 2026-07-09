#!/usr/bin/env bash
# growth-candidate-dedup.sh — drop candidates whose norm_title already appears
# on ANY ledger line (full seen-set: pending/merged/published/rejected/refuted/failed).
set -euo pipefail

CANDS="${1:?usage: growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>}"
LEDGER="${2:?usage: growth-candidate-dedup.sh <candidates.jsonl> <ledger.jsonl>}"
if [ ! -f "$CANDS" ]; then echo "candidates not found: $CANDS" >&2; exit 2; fi

# Build the set of seen norm_titles (empty if ledger absent/empty).
seen='[]'
if [ -f "$LEDGER" ] && [ -s "$LEDGER" ]; then
  seen="$(jq -s '[.[].norm_title]' "$LEDGER")"
fi

# Emit candidates whose norm_title is not in the seen set (exact match via index).
jq -c --argjson seen "$seen" '. as $obj | select(($seen | index($obj.norm_title)) | not)' "$CANDS"
