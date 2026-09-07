#!/usr/bin/env bash
# scripts/route-decide.sh — choose the model for a dispatch, on evidence.
#
# Decision layer over select-model-profile.sh (baseline) + routing-cost.sh
# (measured effective cost). Prints the model id to dispatch on stdout.
#
# PARITY IS STRUCTURAL, NOT INCIDENTAL. The baseline is always computed first,
# and an override only replaces it when the ledger supplies enough evidence AND a
# fitting profile is both cheaper and above the quality floor. With an empty or
# thin ledger this script prints EXACTLY what select-model-profile.sh prints
# today. A router that silently changes routing on a host with no telemetry would
# be worse than the status quo, so "no data" must mean "no change".
#
# Invariants enforced here (see the design doc's invariants section):
#   * Only the kinds named in OVERRIDABLE_KINDS are re-routable, and the list is
#     an ALLOWLIST: any kind not on it — including a kind added to the ledger
#     vocabulary later — falls through to the baseline. A blocklist would open
#     every future kind by default and silently delete this invariant.
#     Deliberately absent, each for its own reason:
#       lgtm-reviewer  a cheap model reviewing its own tier's output degrades
#                      quality invisibly, and the ledger RECORDS that as a
#                      first-pass success — it would reward the pairing.
#       verify-voter   voter independence is a vendor question, not a cost one;
#                      see verify-voter-vendor.sh. Cost-ordering voters would
#                      converge them onto one model and destroy the independence
#                      that makes a second vote worth anything.
#       secaudit-pass  safety gate; never local, never downgraded.
#       spec-decompose spec quality is the upstream bottleneck — a cheap model
#                      here costs N implementer cycles correcting it downstream.
#       growth-lens    unproven against a ledger; add it when there is evidence.
#   * A profile is only a candidate if it FITS the cell on both ordinals
#     (ctx and reasoning); effective cost only orders profiles that already fit.
#   * A local dispatch killed for going silent leaves a `local_overthink_abort`
#     row in the ledger (local-dispatch.sh exit 5), and that row VETOES for the
#     rest of the run: one abort takes its whole cell off local, a second abort
#     from the same profile vetoes it in every cell. The veto outranks the
#     strictly-cheaper gate below — cost is a cost argument, and the cheapest
#     answer to a question is still worthless when the model that gave it
#     produced nothing. Scoping is per run (`run_id` on the row +
#     $AUTOSPEC_RUN_ID here), because a stall is evidence about this run's host
#     load and prompt, not a permanent verdict on the model.
#   * Cold-start exploration is OFF by default and, when enabled, is confined to
#     the lowest-stakes cell. An unproven profile is never explored on real work
#     it could damage.
#
# Usage:
#   route-decide.sh --labels "<comma-separated-issue-labels>"
#                   [--kind <dispatch_kind>] [--print-profile] [--print-effort]
#                   [--explain]
#                   [--profiles-file <path>] [--stats-file <path>]
#
# Exit codes:
#   0  a model id (or profile name) was printed
#   3  nothing resolvable — caller MUST keep its harness-detected TIER_B
#   1  bad arguments
#   2  jq missing (fail-closed)
#
# Environment:
#   AUTOSPEC_ROUTING_POLICY        auto | on | off   (default auto)
#                                  off  -> always the baseline, no ledger read
#                                  on   -> override whenever one is eligible
#                                  auto -> override only when strictly cheaper
#   AUTOSPEC_ROUTING_EXPLORE_PCT   cold-start exploration percent (default 0=off)
#   AUTOSPEC_MODEL_PROFILES        profile catalog
#   AUTOSPEC_ROUTING_LEDGER        ledger read for no-progress aborts
#                                  (default .autospec/routing-ledger.jsonl)
#   AUTOSPEC_RUN_ID                the run those aborts are attributed to; WITHOUT
#                                  it the within-run veto stays off (see above)

set -u

_die() { printf 'route-decide: %s\n' "$1" >&2; exit "${2:-1}"; }

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (fails closed)' 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
LABELS=""
KIND="implementer"
PRINT_PROFILE=0
PRINT_EFFORT=0
EXPLAIN=0
PROFILES_FILE="${AUTOSPEC_MODEL_PROFILES:-$HOME/.autospec/model-profiles.yml}"
STATS_FILE=""
POLICY="${AUTOSPEC_ROUTING_POLICY:-auto}"
EXPLORE_PCT="${AUTOSPEC_ROUTING_EXPLORE_PCT:-0}"

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --labels)        LABELS="${2:-}"; shift 2 ;;
        --kind)          KIND="${2:-}"; shift 2 ;;
        --profiles-file) PROFILES_FILE="${2:-}"; shift 2 ;;
        --stats-file)    STATS_FILE="${2:-}"; shift 2 ;;
        --print-profile) PRINT_PROFILE=1; shift ;;
        --print-effort)  PRINT_EFFORT=1; shift ;;
        --explain)       EXPLAIN=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

_log() { if [ "$EXPLAIN" -eq 1 ]; then printf 'route-decide: %s\n' "$1" >&2; fi }

# Resolve the baseline selector across all three layouts it can live in: the
# installed tree is flat ($AUTOSPEC_SCRIPTS_DIR/*), while in the repo the
# selector is a per-skill script and this one is a top-level script.
SELECTOR=""
for _cand in \
    "$SCRIPT_DIR/select-model-profile.sh" \
    "$SCRIPT_DIR/../skills/autospec-run/scripts/select-model-profile.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/select-model-profile.sh"
do
    if [ -f "$_cand" ]; then SELECTOR="$_cand"; break; fi
done
if [ -z "$SELECTOR" ]; then
    _die 'select-model-profile.sh not found; cannot establish a baseline' 3
fi

# ── baseline (always computed; the override must beat it or stand aside) ───────
baseline_profile="$(AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE" bash "$SELECTOR" --labels "$LABELS" 2>/dev/null || printf '')"
baseline_model="$(AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE" bash "$SELECTOR" --labels "$LABELS" --print-model 2>/dev/null || printf '')"
baseline_effort="$(AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE" bash "$SELECTOR" --labels "$LABELS" --print-effort 2>/dev/null || printf '')"

_emit_baseline() {
    # Effort is per-profile and optional: exit 3 when the catalog does not state
    # one, so the caller keeps its own default rather than being handed a guess.
    if [ "$PRINT_EFFORT" -eq 1 ]; then
        if [ -z "$baseline_effort" ]; then exit 3; fi
        printf '%s\n' "$baseline_effort"
        exit 0
    fi
    if [ "$PRINT_PROFILE" -eq 1 ]; then
        if [ -z "$baseline_profile" ]; then exit 3; fi
        printf '%s\n' "$baseline_profile"
        exit 0
    fi
    if [ -z "$baseline_model" ]; then
        # Fail closed exactly as select-model-profile.sh does: the caller keeps
        # its harness-detected TIER_B rather than being handed a guess.
        exit 3
    fi
    printf '%s\n' "$baseline_model"
    exit 0
}

if [ "$POLICY" = "off" ]; then
    _log "policy=off -> baseline $baseline_profile"
    _emit_baseline
fi

# Allowlist, not a blocklist (see invariants above). These four are the
# high-fan-out read-and-report kinds: they consume a lot of tokens producing
# findings that a later gate re-checks anyway, so a wrong answer is caught
# downstream rather than merged. Every other kind, present or future, is baseline.
OVERRIDABLE_KINDS="implementer explore-researcher refine-lens qa-sweep"

_is_overridable() {
    for _ok in $OVERRIDABLE_KINDS; do
        if [ "$1" = "$_ok" ]; then return 0; fi
    done
    return 1
}

if ! _is_overridable "$KIND"; then
    _log "kind=$KIND is not overridable -> baseline $baseline_profile"
    _emit_baseline
fi

# ── the routing cell ──────────────────────────────────────────────────────────
_cell_ctx=""
_cell_reasoning=""
_old_ifs="$IFS"; IFS=','
for _lbl in $LABELS; do
    _lbl="$(printf '%s' "$_lbl" | tr -d ' ')"
    case "$_lbl" in
        ctx:32k|ctx:small)   _cell_ctx="32k" ;;
        ctx:64k|ctx:medium)  _cell_ctx="64k" ;;
        ctx:120k|ctx:large)  _cell_ctx="120k" ;;
        reasoning:shallow)   _cell_reasoning="shallow" ;;
        reasoning:medium)    _cell_reasoning="medium" ;;
        reasoning:deep)      _cell_reasoning="deep" ;;
    esac
done
IFS="$_old_ifs"

if [ -z "$_cell_ctx" ] || [ -z "$_cell_reasoning" ]; then
    # An unclassified issue has no cell, so there is no evidence to route on.
    _log "no ctx/reasoning cell in labels -> baseline $baseline_profile"
    _emit_baseline
fi

# ── candidates: profiles that FIT the cell on both ordinals ───────────────────
_ord_ctx() { case "$1" in 32k) printf 1 ;; 64k) printf 2 ;; 120k) printf 3 ;; *) printf 0 ;; esac; }
_ord_rsn() { case "$1" in shallow) printf 1 ;; medium) printf 2 ;; deep) printf 3 ;; *) printf 0 ;; esac; }

need_ctx="$(_ord_ctx "$_cell_ctx")"
need_rsn="$(_ord_rsn "$_cell_reasoning")"

candidates=""
# Parse the catalog ONCE into profile<TAB>ctx<TAB>reasoning<TAB>model. Both the
# candidate-fit test and the winner's model lookup read these rows, so there is a
# single place that understands the two YAML layouts.
PROFILE_ROWS=""
if [ -f "$PROFILES_FILE" ]; then
    PROFILE_ROWS="$(awk '
        function lead_ws(s) { match(s, /^ */); return RLENGTH }
        function flush() { if (cur != "") print cur "\t" cx "\t" rs "\t" md "\t" ef "\t" cm }
        {
            line = $0
            sub(/[[:space:]]*#.*$/, "", line)
            if (line ~ /^[[:space:]]*$/) next
            i = lead_ws(line); key = line; sub(/^[[:space:]]+/, "", key)
            if (cur != "" && i <= blocki) { flush(); cur = "" }
            if (key ~ /^[^:]+:[[:space:]]*$/) {
                name = key; sub(/:[[:space:]]*$/, "", name)
                if (name != "profiles") { cur = name; blocki = i; cx = ""; rs = ""; md = ""; ef = ""; cm = "" }
                next
            }
            if (cur == "") next
            v = key; sub(/^[^:]*:[[:space:]]*/, "", v); gsub(/[[:space:]]+$/, "", v)
            if (key ~ /^ctx:/) cx = v
            if (key ~ /^reasoning:/) rs = v
            if (key ~ /^model:/) { gsub(/["\047]/, "", v); md = v }
            if (key ~ /^effort:/) { gsub(/["\047]/, "", v); ef = v }
            # The 6th field is what marks a profile LOCAL: cost_minute is the
            # GPU-minute opportunity price routing-cost.sh uses in place of token
            # prices, so its presence — not a name heuristic — is the definition
            # of "runs on the operator silicon".
            if (key ~ /^cost_minute:/) cm = v
        }
        END { flush() }
    ' "$PROFILES_FILE")"
fi

if [ -n "$PROFILE_ROWS" ]; then
    _rows="$PROFILE_ROWS"
    _old_ifs="$IFS"
    IFS='
'
    for _row in $_rows; do
        _p="$(printf '%s' "$_row" | cut -f1)"
        _pc="$(printf '%s' "$_row" | cut -f2)"
        _pr="$(printf '%s' "$_row" | cut -f3)"
        _pco="$(_ord_ctx "$_pc")"; _pro="$(_ord_rsn "$_pr")"
        if [ "$_pco" -ge "$need_ctx" ] 2>/dev/null && [ "$_pro" -ge "$need_rsn" ] 2>/dev/null; then
            if [ -z "$candidates" ]; then candidates="$_p"; else candidates="$candidates,$_p"; fi
        fi
    done
    IFS="$_old_ifs"
fi

if [ -z "$candidates" ]; then
    _log "no profile fits ctx>=$_cell_ctx reasoning>=$_cell_reasoning -> baseline $baseline_profile"
    _emit_baseline
fi

# ── score and decide ──────────────────────────────────────────────────────────
_cost_args="--kind $KIND --ctx $_cell_ctx --reasoning $_cell_reasoning --candidates $candidates"
if [ -n "$STATS_FILE" ]; then _cost_args="$_cost_args --stats-file $STATS_FILE"; fi

# shellcheck disable=SC2086
scored="$(AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE" bash "$SCRIPT_DIR/routing-cost.sh" $_cost_args 2>/dev/null || printf '[]')"

# ── within-run no-progress veto ───────────────────────────────────────────────
# local-dispatch.sh kills a local dispatch that emits nothing for STALL_SECS and
# records the kill as a `local_overthink_abort` row tagged with the run that
# watched it. Two gates read those rows, both scoped to $AUTOSPEC_RUN_ID:
#
#   one abort in this cell   -> no local profile is offered for this cell again;
#   two aborts from a profile -> that profile is offered in no cell at all.
#
# The first treats a stall as evidence about the CELL (a 32k/shallow prompt that
# makes a 32B model loop will do it again under the same host load) and sends the
# cell to cloud; the second treats it as evidence about the PROFILE. Neither gate
# runs without run identity: an unscoped veto would demote a model permanently on
# rows left by runs that ended days ago, which is history, not a live signal.
VETO_BASELINE=0
VETO_JSON="[]"
_run_id="${AUTOSPEC_RUN_ID:-}"
_abort_ledger="${AUTOSPEC_ROUTING_LEDGER:-$PWD/.autospec/routing-ledger.jsonl}"
if [ -z "$_run_id" ]; then
    _log 'no AUTOSPEC_RUN_ID -> no-progress veto disabled (cannot scope aborts to a run)'
elif [ ! -f "$_abort_ledger" ]; then
    _log "no ledger at $_abort_ledger -> no-progress veto has no aborts to read"
else
    _aborts="$(bash "$SCRIPT_DIR/routing-ledger.sh" --ledger "$_abort_ledger" --show --json 2>/dev/null \
        | jq -c --arg run "$_run_id" '
            [ .[]? | select(.outcome == "local_overthink_abort"
                            and ((.run_id // "")) == $run) ]' 2>/dev/null || printf '[]')"
    case "$_aborts" in ''|null|'[]') _aborts='[]' ;; esac
    if [ "$_aborts" != '[]' ]; then
        # The LOCAL profiles, from the same cost_minute marker routing-cost.sh
        # uses (field 6). Gate 1 counts only aborts filed by one of these: a row
        # written against a cloud profile says nothing about whether the LOCAL
        # tier stalls in that cell, and evicting local on cloud evidence would
        # cost the operator a tier they never broke.
        _local_names=''
        _old_ifs="$IFS"
        IFS='
'
        for _row in $PROFILE_ROWS; do
            if [ -n "$(printf '%s' "$_row" | cut -f6)" ]; then
                _local_names="$_local_names $(printf '%s' "$_row" | cut -f1)"
            fi
        done
        IFS="$_old_ifs"
        _local_json="$(printf '%s\n' $_local_names | jq -R -s '
            split("\n") | map(select(length > 0)) | unique' 2>/dev/null || printf '[]')"

        _veto_names=''
        for _p in $(printf '%s' "$_aborts" | jq -r '
                group_by(.profile) | map(select(length >= 2) | .[0].profile) | .[]' 2>/dev/null); do
            _veto_names="$_veto_names $_p"
        done
        # $row is bound explicitly: inside `($loc | index(...))` the context `.` is
        # $loc, so a bare `.profile` there reads a field off the veto list and the
        # count is silently always 0.
        _cell_aborts="$(printf '%s' "$_aborts" | jq --arg c "$_cell_ctx" --arg r "$_cell_reasoning" --argjson loc "$_local_json" '
            map(. as $row | select($row.cell_ctx == $c and $row.cell_reasoning == $r
                                   and (($loc | index($row.profile)) != null))) | length' 2>/dev/null || printf 0)"
        if [ "$_cell_aborts" -ge 1 ] 2>/dev/null; then
            # A stalled cell gets no local profile for the rest of the run.
            _veto_names="$_veto_names $_local_names"
        fi
        VETO_JSON="$(printf '%s\n' $_veto_names | jq -R -s '
            split("\n") | map(select(length > 0)) | unique')"
        if [ "$VETO_JSON" != '[]' ]; then
            # Marked ineligible rather than dropped from `scored`: the baseline keeps
            # its row, so the strictly-cheaper gate below still has a cost to
            # compare against instead of silently comparing against nothing.
            scored="$(printf '%s' "$scored" | jq -c --argjson v "$VETO_JSON" '
                map(. as $row | if ($v | index($row.profile)) != null
                    then .eligible = false | .reason = "no-progress abort this run"
                    else $row end)' 2>/dev/null || printf '%s' "$scored")"
            if printf '%s' "$VETO_JSON" | jq -e --arg b "$baseline_profile" 'index($b) != null' >/dev/null 2>&1; then
                VETO_BASELINE=1
            fi
            _log "no-progress veto (local_overthink_abort): $VETO_JSON for cell $_cell_ctx/$_cell_reasoning, run $_run_id"
        fi
    fi
fi

winner="$(printf '%s' "$scored" | jq -r 'map(select(.eligible)) | first | .profile // empty')"

if [ -z "$winner" ]; then
    # ── R8 cold start ─────────────────────────────────────────────────────────
    # A profile with no ledger rows scores as ineligible forever: never chosen,
    # so it never earns rows. Bounded exploration breaks that starvation — but
    # only on the LOWEST-stakes cell, and only when the operator opts in
    # (default 0 = off, because exploring on real work is a cost the operator
    # must choose to pay).
    #
    # The draw is a deterministic hash of the labels, not $RANDOM, for two
    # reasons: it is testable, and a retry of the same issue makes the SAME
    # choice instead of flip-flopping mid-issue.
    _explore=0
    case "$EXPLORE_PCT" in
        ''|*[!0-9]*) EXPLORE_PCT=0 ;;
    esac
    if [ "$EXPLORE_PCT" -gt 0 ] 2>/dev/null \
       && [ "$_cell_ctx" = "32k" ] && [ "$_cell_reasoning" = "shallow" ]; then
        _draw="$(printf '%s' "$LABELS" | cksum | awk '{print $1 % 100}')"
        if [ "$_draw" -lt "$EXPLORE_PCT" ] 2>/dev/null; then _explore=1; fi
    fi

    if [ "$_explore" -eq 1 ]; then
        # Cheapest scoreable candidate that is NOT the baseline and has too few
        # samples to be eligible — i.e. the thing we lack evidence about.
        # A vetoed profile is ineligible for a reason that exploration cannot
        # cure: gathering more evidence about the thing that just went silent is
        # the one thing the veto exists to stop.
        _probe="$(printf '%s' "$scored" | jq -r --arg b "$baseline_profile" --argjson v "$VETO_JSON" '
            map(. as $row | select($row.eligible == false and $row.unit != null
                                   and $row.profile != $b
                                   and (($v | index($row.profile)) == null)))
            | first | .profile // empty')"
        if [ -n "$_probe" ]; then
            _log "cold-start exploration (pct=$EXPLORE_PCT, lowest-stakes cell): trying $_probe"
            winner="$_probe"
        fi
    fi

    if [ -z "$winner" ]; then
        if [ "$VETO_BASELINE" -eq 1 ]; then
            # Nothing else fits the cell, so the vetoed baseline is still the best
            # offer available. Say so: the caller cannot tell from the exit code
            # that it was handed a profile this run already watched go silent.
            _log "baseline $baseline_profile is itself vetoed and no alternative fits -> baseline"
        fi
        _log "no eligible profile (thin ledger or all below floor) -> baseline $baseline_profile"
        _emit_baseline
    fi
fi

# An explored profile is deliberately NOT cheaper on paper — its priors are
# pessimistic by construction — so the strictly-cheaper gate must not veto the
# very exploration that exists to gather its evidence.
if [ "${_explore:-0}" -eq 1 ]; then
    _log "exploration bypasses the strictly-cheaper gate"
elif [ "$POLICY" = "auto" ] && [ -n "$baseline_profile" ] && [ "$VETO_BASELINE" -eq 0 ]; then
    # Override only when STRICTLY cheaper than the baseline. If the baseline is
    # unscored (no cost keys) there is nothing to beat, so stand aside.
    _strictly_cheaper="$(printf '%s' "$scored" | jq -r --arg w "$winner" --arg b "$baseline_profile" '
        (map(select(.profile==$b)) | first | .effective_cost) as $bc
        | (map(select(.profile==$w)) | first | .effective_cost) as $wc
        | if $bc == null or $wc == null then "no" elif $wc < $bc then "yes" else "no" end')"
    if [ "$_strictly_cheaper" != "yes" ]; then
        _log "winner $winner not strictly cheaper than baseline $baseline_profile -> baseline"
        _emit_baseline
    fi
elif [ "${VETO_BASELINE:-0}" -eq 1 ]; then
    # The gate is skipped, not failed. Being cheaper is exactly why a stalled
    # local profile is the baseline in the first place, so letting it veto here
    # would resurrect the profile the veto exists to remove.
    _log "baseline $baseline_profile is vetoed by a no-progress abort; strictly-cheaper gate does not resurrect it"
fi

if [ "$winner" = "$baseline_profile" ]; then
    _log "winner equals baseline -> $baseline_profile"
    _emit_baseline
fi

_log "override: $baseline_profile -> $winner (cell $_cell_ctx/$_cell_reasoning)"

if [ "$PRINT_PROFILE" -eq 1 ]; then
    printf '%s\n' "$winner"
    exit 0
fi

# Resolve the winner's model id through the same catalog the baseline uses; if it
# has no model: key there is nothing dispatchable, so keep the baseline.
# Resolve the winner's model id from the SAME parsed rows the candidate-fit test
# used; if the profile has no model: key there is nothing dispatchable, so keep
# the baseline rather than dispatching a profile name as if it were a model.
winner_model=""
_old_ifs="$IFS"
IFS='
'
winner_effort=""
for _row in $PROFILE_ROWS; do
    if [ "$(printf '%s' "$_row" | cut -f1)" = "$winner" ]; then
        winner_model="$(printf '%s' "$_row" | cut -f4)"
        winner_effort="$(printf '%s' "$_row" | cut -f5)"
        break
    fi
done
IFS="$_old_ifs"

if [ -z "$winner_model" ]; then
    _log "winner $winner has no model: key -> baseline"
    _emit_baseline
fi

# Effort follows the SAME winner the model does. Reporting the baseline's effort
# alongside an overridden model would pair a tier with a model it was never
# measured on, which is worse than reporting nothing.
if [ "$PRINT_EFFORT" -eq 1 ]; then
    if [ -z "$winner_effort" ]; then exit 3; fi
    printf '%s\n' "$winner_effort"
    exit 0
fi

printf '%s\n' "$winner_model"
exit 0
