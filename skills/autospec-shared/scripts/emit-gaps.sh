#!/usr/bin/env bash
# emit-gaps.sh — shape filtered review findings into the gap JSON contract.
#
# Input findings JSON: array of objects with at least
#   {dimension, severity, file, line, title, body, verdict, dedupe_key}
# where verdict is "keep" or "false_positive" (the evaluate-findings/critic verdict).
# This shaper keeps only verdict=="keep", assigns a stable gap_id (G<n> in order),
# fills a dedupe_key fallback (title-hash) when absent, and writes the gap contract:
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key}
#
# Usage:
#   emit-gaps.sh --findings <path> --out <path>
#   emit-gaps.sh --help
#
# Environment:
#   AUTOSPEC_SCRIPTS_DIR — sibling scripts dir (default: script dir)
#
# Exit codes:
#   0  success (empty/missing input → empty array written)
#   1  jq missing
#
# Requires: bash 3.2+, jq

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"

FINDINGS=""
OUT=""
REVIEW_METADATA=""
OUTCOMES=""
REVIEW_UNAVAILABLE=0
PHASE55_RUN=""

while [ $# -gt 0 ]; do
  case "$1" in
    --findings) FINDINGS="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --review-metadata) REVIEW_METADATA="$2"; shift 2 ;;
    --outcomes) OUTCOMES="$2"; shift 2 ;;
    --review-unavailable) REVIEW_UNAVAILABLE=1; shift ;;
    --phase55-run) PHASE55_RUN="$2"; shift 2 ;;
    --help|-h)
      printf 'Usage: emit-gaps.sh --findings <path> --out <path>\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

command -v jq >/dev/null 2>&1 || { printf 'emit-gaps: jq required\n' >&2; exit 1; }
[ -n "$OUT" ] || { printf 'emit-gaps: --out required\n' >&2; exit 1; }

append_outcome() {
  [ -n "$OUTCOMES" ] || return 0
  _metadata='{}'
  if [ -n "$REVIEW_METADATA" ] && [ -f "$REVIEW_METADATA" ]; then
    _metadata="$(jq -c 'if type == "object" then . else {} end' "$REVIEW_METADATA" 2>/dev/null || printf '{}')"
  fi
  _high="$(jq '[.[] | select(.severity == "high" or .severity == "critical" or .severity == "blocker")] | length' "$OUT")"
  _total="$(jq 'length' "$OUT")"
  if [ "$REVIEW_UNAVAILABLE" -eq 1 ]; then
    _payload="$(jq -cn --arg run "$PHASE55_RUN" '{schema:1,outcome:"review_unavailable",pr:null,phase55_run:$run}')"
  else
    _payload="$(printf '%s' "$_metadata" | jq -c --argjson high "$_high" --argjson total "$_total" --arg run "$PHASE55_RUN" '
      {
        schema: 1,
        outcome: (if (.pr // null) == null then "unattributed" else "reviewed" end),
        pr: (.pr // null), commit: (.commit // null),
        review_receipt_digest: (.review_receipt_digest // null),
        reviewer_harness: (.reviewer_harness // null),
        reviewer_reasoning: (.reviewer_reasoning // null),
        provider_diversified: (.provider_diversified // null),
        review_risk: (.review_risk // null),
        first_pass_lgtm: (.first_pass_lgtm // null),
        escaped_high_severity: $high, escaped_total: $total,
        review_cost: (.review_cost // 0),
        phase55_run: (if $run != "" then $run else (.phase55_run // "") end)
      } + (if (.supersedes_outcome_digest // "") != "" then
        {supersedes_outcome_digest:.supersedes_outcome_digest} else {} end)')"
  fi
  _canonical="$(printf '%s' "$_payload" | jq -cS '.')"
  _digest="$(gap_sha256 "$_canonical")"
  _row="$(printf '%s' "$_payload" | jq -c --arg digest "$_digest" '. + {outcome_digest:$digest}')"
  mkdir -p "$(dirname "$OUTCOMES")" 2>/dev/null || true
  printf '%s\n' "$_row" >> "$OUTCOMES"
}

if [ "$REVIEW_UNAVAILABLE" -eq 1 ]; then
  jq -n '[{
    gap_id:"G1", dimension:"review-governance", severity:"high", file:"", line:0,
    title:"Phase 5.5 review unavailable",
    body:"The broad post-merge review did not produce an attributable result.",
    dedupe_key:"review_unavailable", outcome:"review_unavailable",
    attribution_status:"unavailable", originating_pr:null, originating_commit:null,
    review_receipt_digest:null, reviewer_harness:null, reviewer_reasoning:null,
    provider_diversified:null, review_risk:null
  }]' > "$OUT"
  append_outcome
  exit 0
fi

if [ -z "$FINDINGS" ] || [ ! -f "$FINDINGS" ] || ! jq -e 'type=="array"' "$FINDINGS" >/dev/null 2>&1; then
  printf '[]\n' > "$OUT"
  append_outcome
  exit 0
fi

# Keep verdict=="keep" (or missing verdict → treat as keep), shape to contract,
# assign G<n> gap_ids in order, and fill dedupe_key fallback with title-hash.
jq -c '[ .[] | select((.verdict // "keep") == "keep") ]' "$FINDINGS" > "$OUT.tmp"

_n="$(jq 'length' "$OUT.tmp")"
printf '[' > "$OUT"
_i=0
while [ "$_i" -lt "$_n" ]; do
  _f="$(jq -c ".[$_i]" "$OUT.tmp")"
  _gid="G$((_i + 1))"
  _dk="$(printf '%s' "$_f" | jq -r '.dedupe_key // empty')"
  if [ -z "$_dk" ]; then
    _title="$(printf '%s' "$_f" | jq -r '.title // ""')"
    _dk="$(gap_title_hash "$_title")"
  fi
  _obj="$(printf '%s' "$_f" | jq -c --arg gid "$_gid" --arg dk "$_dk" '{
    gap_id: $gid,
    dimension: (.dimension // "correctness"),
    severity: (.severity // "low"),
    file: (.file // ""),
    line: (.line // 0),
    title: (.title // ""),
    body: (.body // ""),
    dedupe_key: $dk,
    attribution_status: (if
      (.originating_pr != null) and (.originating_commit != null) and
      (.review_receipt_digest != null) and (.reviewer_harness != null) and
      (.reviewer_reasoning != null) and (.provider_diversified != null) and
      (.review_risk != null) then "attributed" else "unavailable" end),
    originating_pr: (.originating_pr // null),
    originating_commit: (.originating_commit // null),
    review_receipt_digest: (.review_receipt_digest // null),
    reviewer_harness: (.reviewer_harness // null),
    reviewer_reasoning: (.reviewer_reasoning // null),
    provider_diversified: (.provider_diversified // null),
    review_risk: (.review_risk // null)
  }')"
  [ "$_i" -gt 0 ] && printf ',' >> "$OUT"
  printf '%s' "$_obj" >> "$OUT"
  _i=$((_i + 1))
done
printf ']\n' >> "$OUT"
rm -f "$OUT.tmp"
append_outcome
exit 0
