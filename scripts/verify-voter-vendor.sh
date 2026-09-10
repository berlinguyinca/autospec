#!/usr/bin/env bash
# scripts/verify-voter-vendor.sh — pick the VENDOR for the next independent
# reviewer: a verify voter (default role) or a PR reviewer (reviewer role).
#
# A second review is only worth its cost if it can disagree. Two dispatches to
# the same model family share training data, tokenizer, and failure modes, so
# they tend to be wrong together — the one case a second review exists to
# catch. This script therefore chooses from a DIFFERENT vendor than the one
# that produced the work under review: Codex against Claude, Claude against
# Codex.
#
# What this script does NOT do: choose a tier. The reviewer runs at the chosen
# harness's own tier. `verify-voter` is deliberately absent from
# route-decide.sh's overridable allowlist, because cost-ordering voters
# converges them onto the single cheapest model — the exact correlation this
# script exists to break. Vendor is an independence lever; tier is a quality
# lever; they are not the same decision.
#
# Roles:
#   voter (default, the pre-existing contract) —
#       --proposer <vendor>
#   reviewer (issue #3347, guardrails R4: a PR reviewer never shares a vendor
#   with the PR author) —
#       --role reviewer --author <vendor>
#   The authoring vendor is what the implementer ledger row records in
#   `authoring_vendor` (scripts/routing-ledger.sh). `local` is a valid
#   authoring value — a local model implemented the PR (e.g. via
#   local-dispatch.sh) — but is never a candidate: a local-implemented PR must
#   be reviewed by a cloud vendor, and "local" is not a dispatchable review
#   harness in this vocabulary. A claude-authored PR prefers codex, the vendor
#   autospec already depends on for peer review, before any spend tiebreak.
#
# Decision order (each step can only narrow):
#   1. candidate vendors installed on this host (cloud only, never "local")
#   2. minus any vendor named --unavailable  (reactive 429 / quota failover)
#   3. minus the proposer's / author's own vendor   (the independence invariant)
#   4. reviewer role with a claude author: codex, if it survived step 3
#      (a genuinely different vendor, not a second sample of the same one)
#   5. of what remains, the one this repo's routing ledger shows the LEAST
#      spend against, so alternation is self-balancing without a quota API
#
# Step 2 is the load-bearing mechanism, not step 5. scripts/usage-observe.sh
# reports observable=false for all three harnesses — no harness exposes a live
# quota fraction — so remaining budget is not measurable, only inferable. A
# 429 is ground truth; ledger spend is an estimate that is wrong by however
# much the operator used that harness interactively outside autospec. Treat
# step 5 as a tiebreak between vendors that are both fine, never as a quota
# reading.
#
# Usage:
#   verify-voter-vendor.sh --proposer <vendor> [--unavailable <vendor>]...
#                          [--ledger <path>] [--explain]
#   verify-voter-vendor.sh --role reviewer --author <vendor>
#                          [--unavailable <vendor>]... [--ledger <path>] [--explain]
#
# Vendors: claude | codex | opencode
# Authoring vendors: claude | codex | opencode | local
#
# Exit codes:
#   0  a vendor was printed
#   1  usage error
#   3  no INDEPENDENT vendor available — caller keeps its current behaviour
#      (voter role: a same-vendor TIER_B voter; reviewer role: its own TIER_A,
#      never a silent same-vendor reviewer). Fails closed rather than printing
#      the author's own vendor, which would claim an independence it does not
#      have.
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
AUTHOR_VENDORS="claude codex opencode local"
ROLE=
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
        --role)
            if [ $# -lt 2 ]; then _die '--role requires a role'; fi
            ROLE="$2"; shift 2 ;;
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

case "$ROLE" in
    "") ROLE="voter" ;;
    voter|reviewer) ;;
    *) _die "unknown role: $ROLE (known: voter, reviewer)" ;;
esac

if [ "$ROLE" = "reviewer" ]; then
    # R4 (issue #3347): the review-lane selector. The authoring vendor is
    # whatever the implementer ledger row recorded — a cloud harness or
    # `local` when a local model implemented the PR.
    if [ -z "$AUTHOR" ]; then _die '--role reviewer requires --author <vendor> (claude | codex | opencode | local)'; fi
    _author_ok=0
    for _a in $AUTHOR_VENDORS; do
        if [ "$AUTHOR" = "$_a" ]; then _author_ok=1; fi
    done
    if [ "$_author_ok" -ne 1 ]; then _die "unknown author vendor: $AUTHOR (known: claude, codex, opencode, local)"; fi
    if [ -n "$PROPOSER" ]; then _die '--proposer is not valid with --role reviewer; use --author'; fi
else
    if [ -n "$AUTHOR" ]; then _die '--author is only valid with --role reviewer (voter role uses --proposer)'; fi
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
# candidate set and look like "that harness is not installed". In the reviewer
# role the list may name `local`; it is stripped below, because a local model
# never reviews — "local" is an authoring value, not a dispatchable review
# harness.
_candidates=
if [ -n "${AUTOSPEC_VOTER_VENDORS:-}" ]; then
    for _v in $(printf '%s' "$AUTOSPEC_VOTER_VENDORS" | tr ',' ' '); do
        if [ "$ROLE" = "reviewer" ]; then
            _listed=0
            for _a in $AUTHOR_VENDORS; do
                if [ "$_v" = "$_a" ]; then _listed=1; fi
            done
            if [ "$_listed" -ne 1 ]; then _die "unknown vendor in AUTOSPEC_VOTER_VENDORS: $_v"; fi
        elif ! _is_known "$_v"; then
            _die "unknown vendor in AUTOSPEC_VOTER_VENDORS: $_v"
        fi
        if [ "$ROLE" = "reviewer" ] && [ "$_v" = "local" ]; then
            _log "'local' is an authoring value, not a dispatchable review harness — not a reviewer candidate"
            continue
        fi
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

# ── step 3: independence (never the producer's own vendor) ─────────────────────
_independent=
for _v in $_after_failover; do
    if [ "$ROLE" = "reviewer" ]; then
        if [ "$_v" != "$AUTHOR" ]; then _independent="$_independent $_v"; fi
    else
        if [ "$_v" != "$PROPOSER" ]; then _independent="$_independent $_v"; fi
    fi
done
if [ "$ROLE" = "reviewer" ]; then
    _log "independent of author=$AUTHOR: ${_independent:-<none>}"
else
    _log "independent of proposer=$PROPOSER: ${_independent:-<none>}"
fi

if [ -z "$_independent" ]; then
    if [ "$ROLE" = "reviewer" ]; then
        # Reviewer mode is new: a fail-closed review is a guardrail signal the
        # caller must see, so print it regardless of --explain. Voter mode
        # keeps its pinned contract (silent unless --explain, exit code is
        # the signal).
        printf '%s: no independent reviewer vendor (author=%s) -> caller keeps TIER_A rather than a same-vendor reviewer\n' "$PROG" "$AUTHOR" >&2
    else
        _log 'no independent vendor -> caller keeps its current same-vendor voter'
    fi
    exit 3
fi

# ── step 4: reviewer role, claude author -> prefer codex ───────────────────────
# A claude-implemented PR gets codex as its reviewer when codex is available:
# a genuinely different vendor that autospec already depends on for peer
# review. Independence is the property under test; the spend tiebreak below
# applies to everyone else (and to claude authors whose codex is unavailable).
_winner=
if [ "$ROLE" = "reviewer" ] && [ "$AUTHOR" = "claude" ]; then
    for _v in $_independent; do
        if [ "$_v" = "codex" ]; then
            _winner="codex"
            _log "author=claude: codex preferred (genuinely different vendor, the existing peer-review dependency)"
        fi
    done
fi

# ── step 5: least-spent wins (tiebreak only) ───────────────────────────────────
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

if [ -z "$_winner" ]; then
    _winner_spend=
    for _v in $_independent; do
        _s="$(_spend_of "$_v")"
        case "$_s" in ''|*[!0-9]*) _s=0 ;; esac
        _log "spend($_v)=$_s"
        # Strictly-less keeps the comparison total: on a tie the first candidate
        # wins — $KNOWN_VENDORS order when the host was probed, the caller's
        # order when AUTOSPEC_VOTER_VENDORS supplied the list. Either way it is
        # a fixed order, so the choice is deterministic and testable rather than
        # PATH-dependent.
        if [ -z "$_winner" ] || [ "$_s" -lt "$_winner_spend" ]; then
            _winner="$_v"; _winner_spend="$_s"
        fi
    done
    _log "chose $_winner (least spend among independents)"
else
    _log "chose $_winner"
fi

printf '%s\n' "$_winner"
exit 0
