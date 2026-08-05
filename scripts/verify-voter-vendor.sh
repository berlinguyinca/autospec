#!/usr/bin/env bash
# scripts/verify-voter-vendor.sh — pick the VENDOR for the next verify voter.
#
# A second vote is only worth its cost if it can disagree. Two dispatches to the
# same model family share training data, tokenizer, and failure modes, so they
# tend to be wrong together — the one case a verify pass exists to catch. This
# script therefore chooses a voter from a DIFFERENT vendor than the proposer:
# Codex against Claude, Claude against Codex.
#
# What this script does NOT do: choose a tier. The voter runs at the chosen
# harness's own TIER_B. `verify-voter` is deliberately absent from
# route-decide.sh's overridable allowlist, because cost-ordering voters converges
# them onto the single cheapest model — the exact correlation this script exists
# to break. Vendor is an independence lever; tier is a quality lever; they are not
# the same decision.
#
# Decision order (each step can only narrow):
#   1. candidate vendors installed on this host
#   2. minus any vendor named --unavailable  (reactive 429 / quota failover)
#   3. minus the proposer's own vendor       (the independence invariant)
#   4. of what remains, the one this repo's routing ledger shows the LEAST spend
#      against, so alternation is self-balancing without a quota API
#
# Step 2 is the load-bearing mechanism, not step 4. scripts/usage-observe.sh
# reports observable=false for all three harnesses — no harness exposes a live
# quota fraction — so remaining budget is not measurable, only inferable. A 429
# is ground truth; ledger spend is an estimate that is wrong by however much the
# operator used that harness interactively outside autospec. Treat step 4 as a
# tiebreak between vendors that are both fine, never as a quota reading.
#
# Usage:
#   verify-voter-vendor.sh --proposer <vendor> [--unavailable <vendor>]...
#                          [--ledger <path>] [--explain]
#
# Vendors: claude | codex | opencode
#
# Exit codes:
#   0  a vendor was printed
#   1  usage error
#   3  no INDEPENDENT vendor available — caller keeps its current behaviour (a
#      same-vendor TIER_B voter). Fails closed rather than printing the proposer's
#      own vendor, which would claim an independence it does not have.
#
# Environment:
#   AUTOSPEC_VOTER_VENDORS   override host detection with an explicit list
#   AUTOSPEC_ROUTING_LEDGER  ledger path (default .autospec/routing-ledger.jsonl)
#
# bash 3.2+. set -u; if/then/fi one-sided conditionals; no RETURN traps.

set -u

PROG="verify-voter-vendor"
_die() { printf '%s: %s\n' "$PROG" "$1" >&2; exit "${2:-1}"; }

KNOWN_VENDORS="claude codex opencode"
PROPOSER=
UNAVAILABLE=
LEDGER="${AUTOSPEC_ROUTING_LEDGER:-.autospec/routing-ledger.jsonl}"
EXPLAIN=0

_log() { if [ "$EXPLAIN" -eq 1 ]; then printf '%s: %s\n' "$PROG" "$1" >&2; fi }

_is_known() {
    for _k in $KNOWN_VENDORS; do
        if [ "$1" = "$_k" ]; then return 0; fi
    done
    return 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --proposer)
            if [ $# -lt 2 ]; then _die '--proposer requires a vendor'; fi
            PROPOSER="$2"; shift 2 ;;
        --unavailable)
            if [ $# -lt 2 ]; then _die '--unavailable requires a vendor'; fi
            UNAVAILABLE="$UNAVAILABLE $2"; shift 2 ;;
        --ledger)
            if [ $# -lt 2 ]; then _die '--ledger requires a path'; fi
            LEDGER="$2"; shift 2 ;;
        --explain) EXPLAIN=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$PROPOSER" ]; then _die '--proposer is required'; fi
if ! _is_known "$PROPOSER"; then _die "unknown vendor: $PROPOSER"; fi
for _u in $UNAVAILABLE; do
    if ! _is_known "$_u"; then _die "unknown vendor: $_u"; fi
done

# ── step 1: which vendors exist here ──────────────────────────────────────────
# The env override exists so a caller that already knows the fleet (and tests)
# need not depend on PATH. Unknown names in the override are a usage error, not
# something to silently drop: a typo'd vendor would otherwise shrink the
# candidate set and look like "that harness is not installed".
_candidates=
if [ -n "${AUTOSPEC_VOTER_VENDORS:-}" ]; then
    for _v in $(printf '%s' "$AUTOSPEC_VOTER_VENDORS" | tr ',' ' '); do
        if ! _is_known "$_v"; then _die "unknown vendor in AUTOSPEC_VOTER_VENDORS: $_v"; fi
        _candidates="$_candidates $_v"
    done
else
    for _v in $KNOWN_VENDORS; do
        if command -v "$_v" >/dev/null 2>&1; then _candidates="$_candidates $_v"; fi
    done
fi
_log "installed: ${_candidates:-<none>}"

# ── step 2: reactive failover (ground truth) ───────────────────────────────────
_after_failover=
for _v in $_candidates; do
    _skip=0
    for _u in $UNAVAILABLE; do
        if [ "$_v" = "$_u" ]; then _skip=1; fi
    done
    if [ "$_skip" -eq 0 ]; then _after_failover="$_after_failover $_v"; fi
done
_log "after failover: ${_after_failover:-<none>}"

# ── step 3: independence (never the proposer's own vendor) ─────────────────────
_independent=
for _v in $_after_failover; do
    if [ "$_v" != "$PROPOSER" ]; then _independent="$_independent $_v"; fi
done
_log "independent of proposer=$PROPOSER: ${_independent:-<none>}"

if [ -z "$_independent" ]; then
    _log 'no independent vendor -> caller keeps its current same-vendor voter'
    exit 3
fi

# ── step 4: least-spent wins (tiebreak only) ──────────────────────────────────
# Spend is summed from this repo's ledger over ALL dispatch kinds, not just
# verify-voter rows: quota is consumed per harness, so an implementer dispatch
# spends the same budget a voter would. Latest-line-per-dispatch_id, because the
# ledger is append-only and a dispatch may be corrected by a later row.
_spend_of() {
    if [ ! -f "$LEDGER" ]; then printf '0\n'; return 0; fi
    if ! command -v jq >/dev/null 2>&1; then printf '0\n'; return 0; fi
    jq -rs --arg h "$1" '
        [ .[]
          | select(type == "object")
        ] as $rows
        | ($rows | group_by(.dispatch_id) | map(.[-1])) as $latest
        | [ $latest[]
            | select(.harness == $h)
            | ((.input_tokens // 0) + (.output_tokens // 0))
          ] | add // 0
    ' "$LEDGER" 2>/dev/null || printf '0'
}

_winner=
_winner_spend=
for _v in $_independent; do
    _s="$(_spend_of "$_v")"
    case "$_s" in ''|*[!0-9]*) _s=0 ;; esac
    _log "spend($_v)=$_s"
    # Strictly-less keeps the comparison total: on a tie the first candidate wins
    # — $KNOWN_VENDORS order when the host was probed, the caller's order when
    # AUTOSPEC_VOTER_VENDORS supplied the list. Either way it is a fixed order, so
    # the choice is deterministic and testable rather than PATH-dependent.
    if [ -z "$_winner" ] || [ "$_s" -lt "$_winner_spend" ]; then
        _winner="$_v"; _winner_spend="$_s"
    fi
done

_log "chose $_winner (spend=$_winner_spend)"
printf '%s\n' "$_winner"
exit 0
