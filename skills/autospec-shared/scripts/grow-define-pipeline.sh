#!/usr/bin/env bash
# grow-define-pipeline.sh — deterministic G2 pipeline: validate -> dedup ->
# verify (against a supplied verdicts file) -> rank -> slice top-N.
set -euo pipefail

CANDS="${1:?usage: grow-define-pipeline.sh <candidates.jsonl> <verdicts.jsonl> <config.json>}"
VERDICTS="${2:?usage: ...}"
CONFIG="${3:?usage: ...}"
for f in "$CANDS" "$VERDICTS" "$CONFIG"; do
  if [ ! -f "$f" ]; then echo "not found: $f" >&2; exit 2; fi
done

HERE="$(cd "$(dirname "$0")" && pwd)"
MAXN="$(jq -r '.grow.max_issues_per_cycle // 8' "$CONFIG")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# 1. validate: keep only valid candidates (log + drop invalid)
: > "$work/valid.jsonl"
while IFS= read -r line; do
  [ -n "$line" ] || continue
  echo "$line" > "$work/one.json"
  if "$HERE/validate-growth-candidate.sh" "$work/one.json" >/dev/null 2>"$work/err"; then
    echo "$line" >> "$work/valid.jsonl"
  else
    echo "grow-define: dropped invalid candidate: $(cat "$work/err")" >&2
  fi
done < "$CANDS"

# 2. dedup against the ledger
"$HERE/growth-candidate-dedup.sh" "$work/valid.jsonl" "${GROWTH_LEDGER:-.autospec/growth/ledger.jsonl}" > "$work/deduped.jsonl" || true

# 3. verify each survivor against its verdict (fail-closed if absent)
: > "$work/verified.jsonl"
while IFS= read -r line; do
  [ -n "$line" ] || continue
  echo "$line" > "$work/cand.json"
  nt="$(echo "$line" | jq -r '.norm_title')"
  # find verdict by exact norm_title; default fail-closed
  v="$(jq -c --arg n "$nt" 'select(.norm_title==$n)' "$VERDICTS" | tail -1)"
  if [ -z "$v" ]; then
    v='{"real":false,"reason":"no verdict, refused"}'
  fi
  echo "$v" > "$work/verdict.json"
  "$HERE/growth-candidate-verify.sh" "$work/cand.json" "$work/verdict.json" >> "$work/verified.jsonl"
done < "$work/deduped.jsonl"

# 4. rank, 5. slice top-N
if [ ! -s "$work/verified.jsonl" ]; then exit 0; fi
"$HERE/growth-candidate-rank.sh" "$work/verified.jsonl" | jq -c . | head -n "$MAXN"
