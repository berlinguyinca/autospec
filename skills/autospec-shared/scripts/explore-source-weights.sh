#!/usr/bin/env bash
# explore-source-weights.sh — dynamic per-source ranking weights for
# autospec-explore, derived from the outcome ledger via Bayesian smoothing.
#
# This script is the SINGLE SOURCE OF TRUTH for the researcher-source priors.
# It replaces the hardcoded SRC_WEIGHTS dict in explore-research-cycle.sh: the
# research cycle now consumes this script's JSON instead of carrying its own
# table.
#
# Weighting formula (Bayesian-smoothed clean-ship rate):
#
#   base   = (merged_clean + alpha * prior) / (filed + alpha)
#   weight = base * downweight
#
#   prior        = the source's canonical prior (or 0.5 for unknown sources)
#   merged_clean = # proposals from this source that merged clean (--stats)
#   filed        = # proposals from this source that were filed   (--stats)
#   refuted      = # proposals from this source refuted by the verify stage
#                  (--stats; never counted toward filed) — Issue D
#   alpha        = smoothing pseudo-count (default 5)
#
# Refutation-rate down-weighting (Issue D): a source that the adversarial
# verify stage keeps refuting is producing false positives, so it is
# automatically pulled down WITHOUT hand-tuning:
#
#   r          = refuted / (refuted + filed)        (0 when never refuted)
#   downweight = max(FLOOR, 1 - BETA * r)
#
#   BETA  = AUTOSPEC_EXPLORE_REFUTE_BETA  (default 0.5) — strength of the pull.
#   FLOOR = AUTOSPEC_EXPLORE_REFUTE_FLOOR (default 0.25) — a consistently-
#           refuted source is throttled, never starved to 0 (it can recover as
#           clean ships accrue). The multiplier is bounded + monotone in r, so
#           weights CONVERGE rather than oscillate.
#   With no refutations (r=0) downweight==1.0 -> weights are byte-identical to
#   the pre-Issue-D behavior (round-trip parity preserved).
#
# Properties:
#   - No data (filed=0)  -> weight == prior exactly (round-1 parity with the old
#     static table). A missing/empty ledger means "no learning yet": emit priors.
#   - Accumulating clean merges raise a source's weight toward 1; repeated
#     failures pull it down toward 0.
#   - Larger alpha keeps weights closer to the prior (slower to move on data).
#   - Weight is always in [0,1] (merged_clean<=filed, prior in [0,1]); rounded
#     to 4 decimals.
#
# Output sources = the canonical universal + discovery sources UNION any source present in the
# ledger stats. Unknown (non-canonical) sources use the default prior 0.5.
#
# Usage:
#   explore-source-weights.sh [--ledger <path>] [--alpha N] [--json] [--explain]
#     --ledger <path>  forwarded to explore-ledger.sh --stats
#     --alpha  N        smoothing pseudo-count (default 5)
#     --json            (default behavior) emit JSON {source: weight, ...}
#     --explain         also print per-source lines to stderr:
#                         source: filed=F clean=C prior=P -> weight=W
#
# Exit codes:
#   0  ok
#   1  bad arguments
#   2  jq missing (fail-closed — jq is needed to emit/round JSON regardless)
#
# Requires: bash 3.2+, jq, python3. No process substitution (bash 3.2 safe).

set +e

# Canonical prior table — the single source of truth.
CANONICAL_SOURCES="spec-vs-code prior-reports codebase-signals open-issues source-analysis dependency-health internet quality-resilience dogfooding self-leverage style-normalization"
DEFAULT_PRIOR="0.5"

_prior_for() {
  case "$1" in
    spec-vs-code)      echo "1.0" ;;
    prior-reports)     echo "0.9" ;;
    codebase-signals)  echo "0.7" ;;
    dependency-health) echo "0.65" ;;
    open-issues)       echo "0.6" ;;
    source-analysis)   echo "0.5" ;;
    internet)          echo "0.4" ;;
    quality-resilience) echo "0.95" ;;
    dogfooding)        echo "0.9" ;;
    self-leverage)     echo "0.6" ;;
    style-normalization) echo "0.85" ;;
    *)                 echo "$DEFAULT_PRIOR" ;;
  esac
}

_die() { printf 'explore-source-weights: %s\n' "$1" >&2; exit "${2:-1}"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LEDGER_SH="$SCRIPT_DIR/explore-ledger.sh"

LEDGER=""
ALPHA="5"
EXPLAIN=0
# Refutation-rate down-weight tuning (Issue D). Bounded, monotone -> converges.
REFUTE_BETA="${AUTOSPEC_EXPLORE_REFUTE_BETA:-0.5}"
REFUTE_FLOOR="${AUTOSPEC_EXPLORE_REFUTE_FLOOR:-0.25}"

while [ $# -gt 0 ]; do
  case "$1" in
    --ledger)  LEDGER="$2"; shift 2 ;;
    --alpha)   ALPHA="$2"; shift 2 ;;
    --json)    shift ;;            # JSON is the default; accepted for clarity
    --explain) EXPLAIN=1; shift ;;
    -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) _die "unknown argument: $1" 1 ;;
  esac
done

# jq is mandatory: we need it to emit/round JSON regardless of the ledger.
command -v jq >/dev/null 2>&1 || _die "jq not found (required to emit weights JSON)" 2
command -v python3 >/dev/null 2>&1 || _die "python3 not found (required for arithmetic)" 1

# Validate alpha is a positive number.
case "$ALPHA" in
  ''|*[!0-9.]*) _die "invalid --alpha: $ALPHA" 1 ;;
esac
nonzero="$(python3 -c "import sys; a=float(sys.argv[1]); print('ok' if a>0 else 'bad')" "$ALPHA" 2>/dev/null)"
[ "$nonzero" = "ok" ] || _die "invalid --alpha (must be > 0): $ALPHA" 1

# Fetch stats from the ledger. A missing/empty ledger OR any explore-ledger.sh
# error (e.g. exit 2) means "no learning yet" -> empty stats -> priors only.
ledger_args=""
[ -n "$LEDGER" ] && ledger_args="--ledger $LEDGER"
# shellcheck disable=SC2086
stats_json="$(bash "$LEDGER_SH" $ledger_args --stats --json 2>/dev/null)"
rc=$?
if [ "$rc" -ne 0 ] || [ -z "$stats_json" ]; then
  stats_json="{}"
fi
# Guard against non-JSON output.
printf '%s' "$stats_json" | jq -e '.' >/dev/null 2>&1 || stats_json="{}"

# Build the union of canonical sources and any source present in the stats.
stats_sources="$(printf '%s' "$stats_json" | jq -r 'keys[]?' 2>/dev/null)"
all_sources="$CANONICAL_SOURCES $stats_sources"

# Deduplicate while preserving canonical order first.
seen=" "
ordered=""
for s in $all_sources; do
  case "$seen" in
    *" $s "*) ;;
    *) ordered="$ordered $s"; seen="$seen$s " ;;
  esac
done

# Compute each weight and assemble JSON via jq (typed numbers).
jq_args=""
jq_obj="{}"
for s in $ordered; do
  prior="$(_prior_for "$s")"
  filed="$(printf '%s' "$stats_json" | jq -r --arg s "$s" '.[$s].filed // 0')"
  clean="$(printf '%s' "$stats_json" | jq -r --arg s "$s" '.[$s].merged_clean // 0')"
  refuted="$(printf '%s' "$stats_json" | jq -r --arg s "$s" '.[$s].refuted // 0')"
  [ -n "$filed" ] || filed=0
  [ -n "$clean" ] || clean=0
  [ -n "$refuted" ] || refuted=0

  # base   = (clean + alpha*prior) / (filed + alpha)
  # r      = refuted / (refuted + filed)
  # weight = base * max(floor, 1 - beta*r), rounded to 4 decimals.
  weight="$(python3 -c "
import sys
clean=float(sys.argv[1]); filed=float(sys.argv[2]); alpha=float(sys.argv[3]); prior=float(sys.argv[4])
refuted=float(sys.argv[5]); beta=float(sys.argv[6]); floor=float(sys.argv[7])
base=(clean + alpha*prior)/(filed + alpha)
denom=refuted + filed
r=(refuted/denom) if denom>0 else 0.0
downweight=1.0 - beta*r
if downweight<floor: downweight=floor
if downweight>1.0: downweight=1.0
w=base*downweight
if w<0: w=0.0
if w>1: w=1.0
w=round(w,4)
# Emit a canonical JSON number: trim trailing zeros (0.4000 -> 0.4) but keep
# integral values as a single trailing .0 (1.0) so it stays a float literal.
s=('%.4f' % w).rstrip('0').rstrip('.')
print(s if '.' in s else s + '.0')
" "$clean" "$filed" "$ALPHA" "$prior" "$refuted" "$REFUTE_BETA" "$REFUTE_FLOOR")"

  jq_obj="$(printf '%s' "$jq_obj" | jq -c --arg s "$s" --argjson w "$weight" '.[$s] = $w')"

  if [ "$EXPLAIN" -eq 1 ]; then
    # Use jq's normalized number (e.g. 0.8000 -> 0.8) so explain matches JSON.
    weight_norm="$(printf '%s' "$jq_obj" | jq -r --arg s "$s" '.[$s]')"
    printf '%s: filed=%s clean=%s prior=%s -> weight=%s (refuted=%s)\n' \
      "$s" "$filed" "$clean" "$prior" "$weight_norm" "$refuted" >&2
  fi
done

printf '%s\n' "$jq_obj"
exit 0
