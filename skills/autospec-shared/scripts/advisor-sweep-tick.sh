#!/usr/bin/env bash
# advisor-sweep-tick.sh — the single call the end-of-run sweep makes to advance
# advisor self-governance under `policy: auto`.
#
# It does three things, deterministically:
#   1. Resolve policy — only `auto` self-governs (on/off/other → no-op).
#   2. Observe — compute the batch's LGTM-first-pass rate + cost/issue from the
#      main telemetry (advisor-observe.sh).
#   3. Baseline + tick —
#      - if no baseline snapshot exists yet, freeze the current (pre-advisor)
#        metrics as the baseline and stop (first activation);
#      - otherwise feed baseline + observed to advisor-govern.sh tick, which
#        promotes/retracts the active gate set.
#
# Everything is fail-safe: missing telemetry, no signal, or a resolver error
# results in a no-op that never breaks the sweep.
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md §Self-governance
#
# Usage:
#   advisor-sweep-tick.sh [--main-telemetry <f>] [--advisor-telemetry <f>]
#                         [--baseline-file <f>] [--min-samples N] [--json]
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

MAIN_TELEMETRY="${AUTOSPEC_MAIN_TELEMETRY:-$HOME/.autospec/telemetry.jsonl}"
ADVISOR_TELEMETRY="${AUTOSPEC_TELEMETRY_DIR:-.autospec/telemetry}/advisor-escalate.jsonl"
BASELINE_FILE="${AUTOSPEC_ADVISOR_BASELINE_FILE:-.autospec/advisor-baseline.json}"
MIN_SAMPLES="${AUTOSPEC_ADVISOR_MIN_SAMPLES:-20}"

while [ $# -gt 0 ]; do
  case "$1" in
    --main-telemetry) MAIN_TELEMETRY="${2:-}"; shift 2 ;;
    --advisor-telemetry) ADVISOR_TELEMETRY="${2:-}"; shift 2 ;;
    --baseline-file) BASELINE_FILE="${2:-}"; shift 2 ;;
    --min-samples) MIN_SAMPLES="${2:-}"; shift 2 ;;
    --json) shift ;;
    --help|-h) printf 'Usage: advisor-sweep-tick.sh [--main-telemetry f] [--advisor-telemetry f] [--baseline-file f] [--min-samples N]\n'; exit 0 ;;
    *) printf 'advisor-sweep-tick.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

emit() { printf '%s\n' "$1"; exit 0; }

# 1. Governance only runs under policy: auto.
POLICY="$("$SCRIPT_DIR/advisor-config.sh" --key policy 2>/dev/null || printf 'auto')"
if [ "$POLICY" != "auto" ]; then
  emit "$(jq -cn --arg p "$POLICY" '{action:"skip",reason:("policy is "+$p)}')"
fi

# 2. Observe current metrics.
observed="$("$SCRIPT_DIR/advisor-observe.sh" --telemetry "$MAIN_TELEMETRY" 2>/dev/null || printf '')"
[ -n "$observed" ] || emit '{"action":"skip","reason":"observe failed"}'

o_lgtm="$(printf '%s' "$observed" | jq -r '.lgtm_first_pass')"
o_cost="$(printf '%s' "$observed" | jq -r '.cost_per_issue')"
rev_issues="$(printf '%s' "$observed" | jq -r '.reviewer_issues')"

# No reviewer signal this window → hold (never capture a zero baseline, never
# feed 0s to the ratchet where they'd read as a regression).
if [ "$rev_issues" -lt 1 ]; then
  emit '{"action":"hold","reason":"no reviewer signal in telemetry"}'
fi

# 3a. First activation → freeze the (pre-advisor) baseline and stop.
if [ ! -f "$BASELINE_FILE" ]; then
  mkdir -p "$(dirname "$BASELINE_FILE")"
  tmp="$(mktemp "${BASELINE_FILE}.XXXXXX")"
  jq -cn --argjson l "$o_lgtm" --argjson c "$o_cost" \
    '{lgtm_first_pass:$l,cost_per_issue:$c,captured:true}' > "$tmp"
  mv "$tmp" "$BASELINE_FILE"
  emit "$(jq -cn --argjson l "$o_lgtm" --argjson c "$o_cost" \
    '{action:"baseline-captured",baseline:{lgtm_first_pass:$l,cost_per_issue:$c}}')"
fi

# 3b. Baseline exists → tick the ratchet against it.
b_lgtm="$(jq -r '.lgtm_first_pass // 0' "$BASELINE_FILE" 2>/dev/null || printf 0)"
b_cost="$(jq -r '.cost_per_issue // 0' "$BASELINE_FILE" 2>/dev/null || printf 0)"

result="$("$SCRIPT_DIR/advisor-govern.sh" tick \
  --telemetry "$ADVISOR_TELEMETRY" --min-samples "$MIN_SAMPLES" \
  --baseline-lgtm "$b_lgtm" --observed-lgtm "$o_lgtm" \
  --baseline-cost "$b_cost" --observed-cost "$o_cost" --json 2>/dev/null || printf '')"

[ -n "$result" ] || emit '{"action":"skip","reason":"govern tick failed"}'
printf '%s\n' "$result"
