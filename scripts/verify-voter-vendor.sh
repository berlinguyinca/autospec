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
# Reviewer mode (R4, issue #3347): a PR's reviewer must not share a vendor
# with its author. A cheap model reviewing its own tier's output degrades
# quality invisibly, and the routing ledger records that as a first-pass
# success — route-decide.sh names it as the reason lgtm-reviewer is not
# overridable. --author applies this script's independence invariant to that
# lane, with the added rules below. Steps 1-3 are shared with voter mode;
# --author replaces step 4 with a fixed per-author preference order:
#   * author "local" (a local model authored the PR) -> the reviewer must be a
#     cloud vendor: "local" is REMOVED from the candidate set in step 3, never
#     merely deprioritised.
#   * author claude -> codex is preferred when available: autospec already
#     depends on it for peer review, and it is a genuinely different vendor
#     rather than a second sample of the same one.
#   * the order is total, so the ledger spend tiebreak is not consulted here
#     (it is voter-mode-only); the choice is deterministic without a ledger.
#   * no distinct vendor available -> exit 3 and the caller keeps the
#     harness's TIER_A. A same-vendor reviewer is refused, never returned.
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
#   verify-voter-vendor.sh --author <vendor|local> [--unavailable <vendor>]...
#                          [--ledger <path>] [--explain]
#
# Vendors: claude | codex | opencode | local
#   "local" is the local-model vendor (R4, #3347): a PATH probe never finds it,
#   so it is a candidate only when named in AUTOSPEC_VOTER_VENDORS — the caller
#   knows its fleet and says so.
#
# Exit codes:
#   0  a vendor was printed
#   1  usage error
#   3  no INDEPENDENT vendor available — caller keeps its current behaviour (a
#      same-vendor TIER_B voter, or the harness's TIER_A reviewer in --author
#      mode). Fails closed rather than printing the proposer's/author's own
#      vendor, which would claim an independence it does not have.
#
# Environment:
#   AUTOSPEC_VOTER_VENDORS   override host detection with an explicit list
#   AUTOSPEC_ROUTING_LEDGER  ledger path (default .autospec/routing-ledger.jsonl)
#
# bash 3.2+. set -u; if/then/fi one-sided conditionals; no RETURN traps.

set -u

PROG="verify-voter-vendor"
_die() { printf '%s: %s\n' "$PROG" "$1" >&2; exit "${2:-1}"; }

KNOWN_VENDORS="claude codex opencode local"
PROPOSER=
AUTHOR=
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
        --author)
            if [ $# -lt 2 ]; then _die '--author requires a vendor'; fi
            AUTHOR="$2"; shift 2 ;;
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

if [ -n "$PROPOSER" ] && [ -n "$AUTHOR" ]; then
    _die 'pass exactly one of --proposer (verify voter) or --author (PR reviewer)'
fi
if [ -n "$AUTHOR" ]; then
    # "unknown" is the ledger's sentinel for "the authoring vendor was never
    # recorded" (routing-ledger.sh normalizes an absent author_vendor to it).
    # Independence cannot be established against an unknown author, so refuse
    # exactly like a single-vendor host: exit 3, caller keeps TIER_A.
    if [ "$AUTHOR" = "unknown" ]; then
        _log 'author vendor unknown -> no independence can be established'
        exit 3
    fi
    if ! _is_known "$AUTHOR"; then _die "unknown vendor: $AUTHOR"; fi
else
    if [ -z "$PROPOSER" ]; then _die '--proposer is required'; fi
    if ! _is_known "$PROPOSER"; then _die "unknown vendor: $PROPOSER"; fi
fi
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

# ── step 3: independence (never the proposer's/author's own vendor) ───────────
# Exactly one of PROPOSER / AUTHOR is set (validated above), so a single
# exclusion variable covers both modes. For --author local this is what makes
# "local author -> cloud reviewer" a REMOVAL rather than a ranking.
_EXCLUDE="$PROPOSER$AUTHOR"
_independent=
for _v in $_after_failover; do
    if [ "$_v" != "$_EXCLUDE" ]; then _independent="$_independent $_v"; fi
done
if [ -n "$AUTHOR" ]; then
    _log "independent of author=$AUTHOR: ${_independent:-<none>}"
else
    _log "independent of proposer=$PROPOSER: ${_independent:-<none>}"
fi

if [ -z "$_independent" ]; then
    if [ -n "$AUTHOR" ]; then
        _log 'no independent vendor -> caller keeps its TIER_A reviewer'
    else
        _log 'no independent vendor -> caller keeps its current same-vendor voter'
    fi
    exit 3
fi

# ── step 4 (reviewer mode): a fixed per-author preference order ───────────────
# Total, so the choice is deterministic with or without a ledger; the spend
# tiebreak below is deliberately voter-mode-only (see the header).
if [ -n "$AUTHOR" ]; then
    _pref=
    case "$AUTHOR" in
        claude)   _pref="codex opencode local" ;;
        codex)    _pref="claude opencode local" ;;
        opencode) _pref="claude codex local" ;;
        local)    _pref="claude codex opencode" ;;
    esac
    _winner=
    for _v in $_pref; do
        for _i in $_independent; do
            if [ "$_v" = "$_i" ]; then _winner="$_v"; fi
        done
        if [ -n "$_winner" ]; then break; fi
    done
    if [ -z "$_winner" ]; then
        _log 'no independent vendor -> caller keeps its TIER_A reviewer'
        exit 3
    fi
    _log "chose $_winner (preferred vendor for author=$AUTHOR)"
    printf '%s\n' "$_winner"
    exit 0
fi

# ── step 4 (voter mode): least-spent wins (tiebreak only) ─────────────────────
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
