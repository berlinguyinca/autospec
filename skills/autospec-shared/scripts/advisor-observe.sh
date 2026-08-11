#!/usr/bin/env bash
# advisor-observe.sh — derive observed advisor-relevant metrics from autospec's
# main telemetry JSONL. These feed advisor-govern.sh's promote/retract decision.
#
# Metrics (computed with the SAME formulas as gen-telemetry-dashboard.sh):
#   lgtm_first_pass  — fraction (0..1) of reviewer issues whose FIRST reviewer
#                      dispatch (by ts) had cache_read > 0 (warm-cache first pass)
#   cost_per_issue   — mean total (input+output) tokens per distinct issue across
#                      all roles in the main telemetry (implementer + reviewer +
#                      …); the advisor's own bounded call cost is tracked
#                      separately in advisor-escalate.jsonl and not included here
#   reviewer_issues  — distinct issues with a reviewer dispatch
#   issues           — distinct issues total
#
# Fail-safe: a missing/empty/garbled telemetry file yields zeroed metrics and
# exit 0 (never blocks the sweep). Malformed lines are skipped.
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md §Self-governance
#
# Usage: advisor-observe.sh --outcomes <review-outcomes-jsonl> [--json]
#        advisor-observe.sh --telemetry <legacy-main-jsonl> [--json]
set -eu

TELEMETRY=""
OUTCOMES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --outcomes) OUTCOMES="${2:-}"; shift 2 ;;
    --json) shift ;;   # JSON is the only output form
    --help|-h) printf 'Usage: advisor-observe.sh --telemetry <main-jsonl> [--json]\n'; exit 0 ;;
    *) printf 'advisor-observe.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

zeroed() { printf '{"escaped_high_rate":0,"escaped_total_rate":0,"cost_per_reviewed_pr":0,"attributed_reviewed_prs":0,"first_pass_lgtm":0,"review_unavailable":false,"lgtm_first_pass":0,"cost_per_issue":0,"reviewer_issues":0,"issues":0}\n'; }

if [ -n "$OUTCOMES" ]; then
  [ -f "$OUTCOMES" ] || { zeroed; exit 0; }
  out="$(jq -R 'fromjson? // empty' "$OUTCOMES" 2>/dev/null | jq -s '
    map(select(type == "object")) as $rows
    | ($rows | map(.supersedes_outcome_digest // empty)) as $superseded
    | ($rows | map(select((.outcome_digest // "") as $id | ($superseded | index($id) | not)))) as $effective
    | ($effective | any(.outcome == "review_unavailable")) as $unavailable
    | ($effective | map(select(
        ((.outcome // "reviewed") == "reviewed") and
        (.pr | type == "number") and
        (.commit | type == "string" and length > 0) and
        (.review_receipt_digest | type == "string" and length > 0) and
        (.reviewer_harness | type == "string" and length > 0) and
        (.reviewer_reasoning | type == "string" and length > 0) and
        (.provider_diversified | type == "boolean") and
        (.review_risk | type == "string" and length > 0)
      ))) as $attributed
    | ($attributed | length) as $n
    | {
        escaped_high_rate: (if $n > 0 then (($attributed | map(.escaped_high_severity // 0) | add // 0) / $n) else 0 end),
        escaped_total_rate: (if $n > 0 then (($attributed | map(.escaped_total // 0) | add // 0) / $n) else 0 end),
        cost_per_reviewed_pr: (if $n > 0 then (($attributed | map(.review_cost // 0) | add // 0) / $n) else 0 end),
        attributed_reviewed_prs: $n,
        first_pass_lgtm: (if $n > 0 then (($attributed | map(select(.first_pass_lgtm == true)) | length) / $n) else 0 end),
        review_unavailable: $unavailable
      }
  ' 2>/dev/null)" || true
  [ -n "$out" ] || { zeroed; exit 0; }
  printf '%s\n' "$out"
  exit 0
fi

if [ -z "$TELEMETRY" ] || [ ! -f "$TELEMETRY" ]; then
  zeroed; exit 0
fi

# Parse defensively (skip malformed lines), then aggregate.
out="$(jq -R 'fromjson? // empty' "$TELEMETRY" 2>/dev/null | jq -s '
  ([.[] | select(.role == "reviewer")] | group_by(.issue)) as $rev
  | ($rev | length) as $reviewer_issues
  | ($rev | map(sort_by(.ts) | .[0]) | map(select((.cache_read_input_tokens // 0) > 0)) | length) as $hits
  | (group_by(.issue) | length) as $issues
  | ([.[] | (.input_tokens // 0) + (.output_tokens // 0)] | add // 0) as $total
  | {
      lgtm_first_pass: (if $reviewer_issues > 0 then ($hits / $reviewer_issues) else 0 end),
      cost_per_issue:  (if $issues > 0 then ($total / $issues) else 0 end),
      reviewer_issues: $reviewer_issues,
      issues: $issues
    }
' 2>/dev/null)" || true

if [ -z "$out" ]; then
  zeroed; exit 0
fi
printf '%s\n' "$out"
