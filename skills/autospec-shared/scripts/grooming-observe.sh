#!/usr/bin/env bash
# grooming-observe.sh — derive the observed backlog-grooming clean-merge rate
# vs a baseline from autospec's telemetry JSONL. Feeds grooming-govern.sh's
# promote/retract decision.
#
# A record is "template-groom" when .template_groomed == true — these are the
# LLM template-fill records the canary→auto ratchet governs. Everything else
# is baseline (ambient/non-template-groom population).
#
# A record is "resolved" when it carries a reconciled `outcome` (non-null),
# or — for pre-enhancement (back-compat) records with no `outcome` field at
# all — when it carries the legacy `reverted`/`reopened` shape. Unresolved
# records (outcome explicitly null, no closing verdict yet) are excluded from
# both samples and rate: never-closed issues must not count as clean or
# unclean.
#
# is_clean: when `outcome` is present, clean iff outcome == "clean". For
# back-compat records without `outcome`, fall back to v1's rule: no revert,
# no reopen, and no escalate:human label/verdict.
#
# Metrics:
#   groomed_clean_merge_rate  — fraction (0..1) of resolved template-groom
#                               records that were clean
#   baseline_clean_merge_rate — fraction (0..1) of resolved non-template-groom
#                               records that were clean
#   samples                   — count of resolved template-groom records (the
#                               promotion sample floor)
#   baseline_samples          — count of resolved non-template-groom records
#                               (govern's widen-guard: never widen without a
#                               real baseline population)
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

# Parse defensively (skip malformed lines), then aggregate. See header comment
# for the template-groom/baseline partition and reconciled-outcome semantics.
out="$(jq -R 'fromjson? // empty' "$TELEMETRY" 2>/dev/null | jq -s '
  def is_template_groom: (.template_groomed == true);
  def is_resolved: (has("outcome") and (.outcome != null))
                   or ((has("outcome") | not) and (has("reverted") or has("reopened")));
  def is_clean:
    if (has("outcome") and (.outcome != null)) then (.outcome == "clean")
    else
      ((.reverted != true) and (.reopened != true)
       and ((.labels // []) | index("escalate:human") | not)
       and (.verdict != "escalate:human"))
    end;
  ([.[] | select(is_resolved)]) as $r
  | ([$r[] | select(is_template_groom)]) as $g
  | ([$r[] | select(is_template_groom | not)]) as $b
  | ($g | length) as $gn | ($b | length) as $bn
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
