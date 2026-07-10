#!/usr/bin/env bash
# grooming-observe.sh — derive the observed backlog-grooming clean-merge rate
# vs a baseline from autospec's telemetry JSONL. Feeds grooming-govern.sh's
# promote/retract decision.
#
# A groomed issue is "clean" if its PR merged with no revert, no reopen, and
# no escalate:human label/verdict. Baseline is computed the same way over
# ungroomed (non-grooming-sourced) issues, so the ratchet only promotes when
# grooming performs at least as well as the ambient issue population.
#
# Metrics:
#   groomed_clean_merge_rate  — fraction (0..1) of groomed issues that were clean
#   baseline_clean_merge_rate — fraction (0..1) of ungroomed issues that were clean
#   samples                   — count of groomed issues (the promotion sample floor)
#   baseline_samples          — count of ungroomed issues (govern's widen-guard:
#                               never widen without a real baseline population)
#
# Fail-safe: a missing/empty/garbled telemetry file yields zeroed metrics and
# exit 0 (never blocks the sweep). Malformed lines are skipped.
#
# Usage: grooming-observe.sh --telemetry <jsonl> [--json]
set -eu

TELEMETRY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --json) shift ;;   # JSON is the only output form
    --help|-h) printf 'Usage: grooming-observe.sh --telemetry <jsonl> [--json]\n'; exit 0 ;;
    *) printf 'grooming-observe.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

zeroed() { printf '{"groomed_clean_merge_rate":0,"baseline_clean_merge_rate":0,"samples":0,"baseline_samples":0}\n'; }

if [ -z "$TELEMETRY" ] || [ ! -f "$TELEMETRY" ]; then
  zeroed; exit 0
fi

# Parse defensively (skip malformed lines), then aggregate. A record is
# considered "groomed" when .groomed == true (or .source == "grooming");
# it is "clean" when it lacks revert/reopen and its verdict/labels don't
# include escalate:human.
out="$(jq -R 'fromjson? // empty' "$TELEMETRY" 2>/dev/null | jq -s '
  def is_groomed: (.groomed == true) or (.source == "grooming");
  def is_clean:
    (.reverted != true)
    and (.reopened != true)
    and ((.labels // []) | index("escalate:human") | not)
    and (.verdict != "escalate:human");
  ([.[] | select(is_groomed)]) as $g
  | ([.[] | select(is_groomed | not)]) as $b
  | ($g | length) as $gn
  | ($b | length) as $bn
  | ([$g[] | select(is_clean)] | length) as $gc
  | ([$b[] | select(is_clean)] | length) as $bc
  | {
      groomed_clean_merge_rate:  (if $gn > 0 then ($gc / $gn) else 0 end),
      baseline_clean_merge_rate: (if $bn > 0 then ($bc / $bn) else 0 end),
      samples: $gn,
      baseline_samples: $bn
    }
' 2>/dev/null)" || true

if [ -z "$out" ]; then
  zeroed; exit 0
fi
printf '%s\n' "$out"
