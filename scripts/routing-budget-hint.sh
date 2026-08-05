#!/usr/bin/env bash
# scripts/routing-budget-hint.sh — turn remaining token budget into a routing bias.
#
# R10: coordinate with the usage governor, do not duplicate it.
# `autonomous-usage-governor.sh` decides whether to PARK, and keeps sole authority
# over that call. This script never parks and never overrides a park; it only
# answers a narrower question the governor does not ask:
#
#   given how much budget is left, how much should paid tiers be penalised?
#
# A learned router gives the governor a better option than stopping: as the
# budget shrinks, weight paid tiers up so eligible work shifts to local profiles
# and the runway extends. Park still arrives — just later.
#
# The multiplier applies ONLY to cloud-priced profiles (cost_in/cost_out). Local
# profiles are priced in wall-clock (cost_minute) and consume no token budget, so
# scaling them would be incoherent.
#
# Output (--json): {used_pct, remaining_pct, cloud_multiplier, hint}
#   hint: normal | prefer-cheap | prefer-local
#
# Bands (deliberately coarse — this is a bias, not a cliff):
#   remaining > 50%   -> 1.0  normal        (no distortion at all)
#   50% .. 25%        -> 1.5  prefer-cheap
#   25% .. 10%        -> 2.5  prefer-cheap
#   below 10%         -> 4.0  prefer-local  (stretch the last of the budget)
#
# Fail-open by design: an unreadable ledger yields multiplier 1.0 (`normal`), so a
# telemetry gap can never distort routing. Exit is always 0 — a hint must not be
# able to break a caller.
#
# Usage:
#   routing-budget-hint.sh [--json] [--used-pct N] [--repo-dir <dir>]
#
# Environment:
#   AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  budget denominator (default 10000000)
#   AUTOSPEC_ROUTING_BUDGET_HINT         force a hint (normal|prefer-cheap|prefer-local)

set -u

JSON=0
USED_PCT=""
REPO_DIR="."
LIFETIME="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-10000000}"

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --json)     JSON=1; shift ;;
        --used-pct) USED_PCT="${2:-}"; shift 2 ;;
        --repo-dir) REPO_DIR="${2:-}"; shift 2 ;;
        *) shift ;;
    esac
done

_emit() {
    _used="$1"; _mult="$2"; _hint="$3"
    _rem="$(awk -v u="$_used" 'BEGIN{r=100-u; if(r<0)r=0; printf "%.1f", r}')"
    if [ "$JSON" -eq 1 ]; then
        printf '{"used_pct":%s,"remaining_pct":%s,"cloud_multiplier":%s,"hint":"%s"}\n' \
            "$_used" "$_rem" "$_mult" "$_hint"
    else
        printf '%s\n' "$_mult"
    fi
    exit 0
}

# An explicit override wins: lets the conductor pin a bias for a whole run.
case "${AUTOSPEC_ROUTING_BUDGET_HINT:-}" in
    normal)       _emit 0 1.0 normal ;;
    prefer-cheap) _emit 0 2.5 prefer-cheap ;;
    prefer-local) _emit 0 4.0 prefer-local ;;
esac

# Derive used-percent: caller-supplied, else from the spend ledger.
if [ -z "$USED_PCT" ]; then
    _ledger_sh=""
    for _c in "$(dirname "$0")/autonomous-spend-ledger.sh" \
              "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autonomous-spend-ledger.sh"; do
        if [ -f "$_c" ]; then _ledger_sh="$_c"; break; fi
    done
    if [ -n "$_ledger_sh" ] && command -v jq >/dev/null 2>&1; then
        _status="$(bash "$_ledger_sh" status --repo-dir "$REPO_DIR" 2>/dev/null || printf '{}')"
        _tokens="$(printf '%s' "$_status" | jq -r '.tokens // 0' 2>/dev/null || printf 0)"
        case "$_tokens" in ''|*[!0-9]*) _tokens=0 ;; esac
        if [ "$LIFETIME" -gt 0 ] 2>/dev/null; then
            USED_PCT="$(awk -v t="$_tokens" -v l="$LIFETIME" 'BEGIN{printf "%.1f", (t/l)*100}')"
        fi
    fi
fi

# Fail open: no readable signal means no distortion.
if [ -z "$USED_PCT" ]; then
    _emit 0 1.0 normal
fi

_band="$(awk -v u="$USED_PCT" 'BEGIN{
    r = 100 - u
    if (r > 50)      print "1.0 normal"
    else if (r > 25) print "1.5 prefer-cheap"
    else if (r > 10) print "2.5 prefer-cheap"
    else             print "4.0 prefer-local"
}')"
_emit "$USED_PCT" "$(printf '%s' "$_band" | awk '{print $1}')" "$(printf '%s' "$_band" | awk '{print $2}')"
