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
# Usage: advisor-observe.sh --telemetry <main-jsonl> [--json]
set -eu

TELEMETRY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --json) shift ;;   # JSON is the only output form
    --help|-h) printf 'Usage: advisor-observe.sh --telemetry <main-jsonl> [--json]\n'; exit 0 ;;
    *) printf 'advisor-observe.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

zeroed() { printf '{"lgtm_first_pass":0,"cost_per_issue":0,"reviewer_issues":0,"issues":0}\n'; }

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
