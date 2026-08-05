#!/usr/bin/env bash
# scripts/routing-ledger.sh — append-only JSONL outcome ledger for model routing.
#
# Records what each dispatch COST and how it turned out, so routing can be scored
# on measured effective cost instead of sticker price. Contract deliberately
# mirrors skills/autospec-shared/scripts/explore-ledger.sh (append-only, outcome
# enum, --stats, --show, --validate, --rebuild-shaped reader semantics) so there
# is one ledger idiom in the repo rather than two.
#
# Record schema (all keys required for --append):
#   {dispatch_id, ts, dispatch_kind, profile, model, harness, issue,
#    cell_ctx, cell_reasoning, input_tokens, output_tokens, cached_tokens,
#    wall_clock_ms, retries, escalated, outcome, reason}
#
#     dispatch_id     string, unique per dispatch attempt (NOT per issue — an
#                     issue can be dispatched many times)
#     dispatch_kind   implementer | lgtm-reviewer | explore-researcher |
#                     verify-voter | refine-lens | qa-sweep | secaudit-pass |
#                     growth-lens | spec-decompose
#                     Present from the FIRST record on purpose: retrofitting a
#                     key dimension into an append-only ledger would invalidate
#                     every historical row and every stats query.
#     profile         model-profiles.yml profile name that served the dispatch
#     model           concrete model id actually dispatched
#     cell_ctx        32k | 64k | 120k        (the routing cell's ctx ordinal)
#     cell_reasoning  shallow | medium | deep  (the routing cell's tier)
#     cached_tokens   prompt-cache hits; feeds the cache-penalty term, which is
#                     the difference between a cheap model and a cheap dispatch
#     escalated       true when the dispatch pulled in a stronger advisor/tier
#     outcome         pending | merged_clean | lgtm_first_pass | retried_ok |
#                     escalated | qa_failed | reverted | abandoned
#
# Append-only audit trail: --update-outcome appends a NEW copy of the record with
# an updated outcome/reason/ts rather than rewriting history. Readers (--show /
# --stats) take the LATEST line per dispatch_id.
#
# Usage:
#   routing-ledger.sh --append '<json-object>'
#   routing-ledger.sh --update-outcome <dispatch_id> <outcome> [reason]
#   routing-ledger.sh --stats [--json]
#   routing-ledger.sh --show [--profile <name>] [--kind <dispatch_kind>] [--json]
#   routing-ledger.sh --validate [<file>]
#   routing-ledger.sh -h | --help
#
# Ledger path (precedence): --ledger <path> > $AUTOSPEC_ROUTING_LEDGER
#   > .autospec/routing-ledger.jsonl
#
# Exit codes:
#   0  ok / valid
#   1  invalid object/line, bad arguments, or --update-outcome id not found
#   2  jq missing (fail-closed — this is a data-integrity tool)
#
# Requires bash 3.2+ and jq. jq is MANDATORY and the script fails closed without
# it: silently degrading a data-integrity tool is worse than refusing to run.

set -u

ALLOWED_OUTCOMES="pending merged_clean lgtm_first_pass retried_ok escalated qa_failed reverted abandoned"
ALLOWED_KINDS="implementer lgtm-reviewer explore-researcher verify-voter refine-lens qa-sweep secaudit-pass growth-lens spec-decompose"
ALLOWED_CTX="32k 64k 120k"
ALLOWED_REASONING="shallow medium deep"

REQUIRED_KEYS="dispatch_id ts dispatch_kind profile model harness issue cell_ctx cell_reasoning input_tokens output_tokens cached_tokens wall_clock_ms retries escalated outcome reason"

_usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }
_die() { printf 'routing-ledger: %s\n' "$1" >&2; exit "${2:-1}"; }

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (data-integrity tool, fails closed)' 2
fi

LEDGER="${AUTOSPEC_ROUTING_LEDGER:-.autospec/routing-ledger.jsonl}"
MODE=""
JSON_OUT=0
FILTER_PROFILE=""
FILTER_KIND=""
ARG1=""; ARG2=""; ARG3=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) _usage; exit 0 ;;
        --ledger)
            if [ $# -lt 2 ]; then _die '--ledger requires a path'; fi
            LEDGER="$2"; shift 2 ;;
        --json) JSON_OUT=1; shift ;;
        --profile)
            if [ $# -lt 2 ]; then _die '--profile requires a name'; fi
            FILTER_PROFILE="$2"; shift 2 ;;
        --kind)
            if [ $# -lt 2 ]; then _die '--kind requires a dispatch_kind'; fi
            FILTER_KIND="$2"; shift 2 ;;
        --append)
            if [ $# -lt 2 ]; then _die '--append requires a JSON object'; fi
            MODE="append"; ARG1="$2"; shift 2 ;;
        --update-outcome)
            if [ $# -lt 3 ]; then _die '--update-outcome requires <dispatch_id> <outcome>'; fi
            MODE="update"; ARG1="$2"; ARG2="$3"
            shift 3
            if [ $# -gt 0 ]; then
                case "$1" in -*) ;; *) ARG3="$1"; shift ;; esac
            fi ;;
        --stats)    MODE="stats"; shift ;;
        --show)     MODE="show"; shift ;;
        --validate)
            MODE="validate"
            shift
            if [ $# -gt 0 ]; then
                case "$1" in -*) ;; *) ARG1="$1"; shift ;; esac
            fi ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$MODE" ]; then
    _usage >&2
    exit 1
fi

_in_list() {
    for _w in $2; do
        if [ "$_w" = "$1" ]; then return 0; fi
    done
    return 1
}

# _validate_object <json> — echo nothing on success; print reason + fail otherwise.
_validate_object() {
    _obj="$1"
    if ! printf '%s' "$_obj" | jq -e 'type == "object"' >/dev/null 2>&1; then
        printf 'not a JSON object\n'
        return 1
    fi
    for _k in $REQUIRED_KEYS; do
        if ! printf '%s' "$_obj" | jq -e --arg k "$_k" 'has($k)' >/dev/null 2>&1; then
            printf 'missing required key: %s\n' "$_k"
            return 1
        fi
    done
    _oc="$(printf '%s' "$_obj" | jq -r '.outcome')"
    if ! _in_list "$_oc" "$ALLOWED_OUTCOMES"; then
        printf 'invalid outcome: %s\n' "$_oc"
        return 1
    fi
    _dk="$(printf '%s' "$_obj" | jq -r '.dispatch_kind')"
    if ! _in_list "$_dk" "$ALLOWED_KINDS"; then
        printf 'invalid dispatch_kind: %s\n' "$_dk"
        return 1
    fi
    _cx="$(printf '%s' "$_obj" | jq -r '.cell_ctx')"
    if ! _in_list "$_cx" "$ALLOWED_CTX"; then
        printf 'invalid cell_ctx: %s\n' "$_cx"
        return 1
    fi
    _cr="$(printf '%s' "$_obj" | jq -r '.cell_reasoning')"
    if ! _in_list "$_cr" "$ALLOWED_REASONING"; then
        printf 'invalid cell_reasoning: %s\n' "$_cr"
        return 1
    fi
    _validate_counters "$_obj"
}

# _validate_counters <json> — numeric/boolean half of the record contract.
_validate_counters() {
    _obj="$1"
    # Counters must be non-negative numbers, never strings: the cost formula
    # divides by them and a string would silently poison every derived weight.
    if ! printf '%s' "$_obj" | jq -e '
        (.input_tokens|type=="number") and (.output_tokens|type=="number") and
        (.cached_tokens|type=="number") and (.wall_clock_ms|type=="number") and
        (.retries|type=="number") and
        (.input_tokens>=0) and (.output_tokens>=0) and (.cached_tokens>=0) and
        (.wall_clock_ms>=0) and (.retries>=0)' >/dev/null 2>&1; then
        printf 'token/wall-clock/retry counters must be non-negative numbers\n'
        return 1
    fi
    if ! printf '%s' "$_obj" | jq -e '.escalated|type=="boolean"' >/dev/null 2>&1; then
        printf 'escalated must be a boolean\n'
        return 1
    fi
    # cached_tokens is a subset of input_tokens; a ratio above 1 means the caller
    # is double-counting and would produce a cache penalty below its true floor.
    if ! printf '%s' "$_obj" | jq -e '.cached_tokens <= .input_tokens' >/dev/null 2>&1; then
        printf 'cached_tokens may not exceed input_tokens\n'
        return 1
    fi
    return 0
}

# Latest line per dispatch_id, preserving first-seen order.
_latest_records() {
    if [ ! -f "$LEDGER" ]; then
        printf '[]'
        return 0
    fi
    jq -s 'map(select(type=="object" and has("dispatch_id")))
           | group_by(.dispatch_id)
           | map(.[-1])' "$LEDGER" 2>/dev/null || printf '[]'
}

case "$MODE" in
    append)
        if ! _reason="$(_validate_object "$ARG1")"; then
            _die "$_reason"
        fi
        _dir="$(dirname "$LEDGER")"
        if [ ! -d "$_dir" ]; then mkdir -p "$_dir"; fi
        printf '%s\n' "$(printf '%s' "$ARG1" | jq -c '.')" >> "$LEDGER"
        exit 0
        ;;

    update)
        if ! _in_list "$ARG2" "$ALLOWED_OUTCOMES"; then
            _die "invalid outcome: $ARG2"
        fi
        if [ ! -f "$LEDGER" ]; then
            _die "--update-outcome: dispatch_id not found: $ARG1"
        fi
        _prev="$(_latest_records | jq -c --arg id "$ARG1" '.[] | select(.dispatch_id==$id)')"
        if [ -z "$_prev" ]; then
            _die "--update-outcome: dispatch_id not found: $ARG1"
        fi
        # Append a NEW record rather than rewriting: the ledger is an audit trail.
        printf '%s' "$_prev" | jq -c \
            --arg oc "$ARG2" --arg rs "$ARG3" \
            --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            '.outcome=$oc | .ts=$ts | (if $rs != "" then .reason=$rs else . end)' >> "$LEDGER"
        exit 0
        ;;

    validate)
        _file="${ARG1:-$LEDGER}"
        if [ ! -f "$_file" ]; then
            # A missing ledger is "no data yet", not corruption.
            exit 0
        fi
        _lineno=0
        _rc=0
        while IFS= read -r _line || [ -n "$_line" ]; do
            _lineno=$((_lineno + 1))
            if [ -z "$_line" ]; then continue; fi
            if ! _reason="$(_validate_object "$_line")"; then
                printf 'routing-ledger: %s:%d: %s\n' "$_file" "$_lineno" "$_reason" >&2
                _rc=1
            fi
        done < "$_file"
        exit "$_rc"
        ;;

    show)
        _recs="$(_latest_records)"
        if [ -n "$FILTER_PROFILE" ]; then
            _recs="$(printf '%s' "$_recs" | jq -c --arg p "$FILTER_PROFILE" '[.[]|select(.profile==$p)]')"
        fi
        if [ -n "$FILTER_KIND" ]; then
            _recs="$(printf '%s' "$_recs" | jq -c --arg k "$FILTER_KIND" '[.[]|select(.dispatch_kind==$k)]')"
        fi
        if [ "$JSON_OUT" -eq 1 ]; then
            printf '%s' "$_recs" | jq '.'
        else
            printf '%s' "$_recs" | jq -r '.[] |
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\t\(.outcome)\tretries=\(.retries)\tms=\(.wall_clock_ms)"'
        fi
        exit 0
        ;;

    stats)
        # One row per (dispatch_kind, profile, cell). These are exactly the
        # coordinates the effective-cost formula scores, so the shape is the
        # contract: routing-cost.sh consumes this and nothing else.
        _stats="$(_latest_records | jq '
            map(select(.outcome != "pending"))
            | group_by([.dispatch_kind, .profile, .cell_ctx, .cell_reasoning])
            | map({
                dispatch_kind: .[0].dispatch_kind,
                profile:       .[0].profile,
                cell_ctx:      .[0].cell_ctx,
                cell_reasoning:.[0].cell_reasoning,
                dispatches:    length,
                first_pass:    (map(select(.outcome=="merged_clean" or .outcome=="lgtm_first_pass")) | length),
                failed:        (map(select(.outcome=="qa_failed" or .outcome=="reverted" or .outcome=="abandoned")) | length),
                escalations:   (map(select(.escalated)) | length),
                retries_total: (map(.retries) | add // 0),
                input_tokens:  (map(.input_tokens) | add // 0),
                output_tokens: (map(.output_tokens) | add // 0),
                cached_tokens: (map(.cached_tokens) | add // 0),
                wall_clock_ms: (map(.wall_clock_ms) | add // 0)
              })
            | map(. + {
                first_pass_rate: (if .dispatches > 0 then (.first_pass / .dispatches) else 0 end),
                failure_rate:    (if .dispatches > 0 then (.failed / .dispatches) else 0 end),
                escalation_rate: (if .dispatches > 0 then (.escalations / .dispatches) else 0 end),
                mean_retries:    (if .dispatches > 0 then (.retries_total / .dispatches) else 0 end),
                cache_hit_ratio: (if .input_tokens > 0 then (.cached_tokens / .input_tokens) else 0 end),
                mean_wall_clock_ms: (if .dispatches > 0 then (.wall_clock_ms / .dispatches) else 0 end)
              })')"
        if [ "$JSON_OUT" -eq 1 ]; then
            printf '%s' "$_stats" | jq '.'
        else
            printf '%s' "$_stats" | jq -r '.[] |
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\tn=\(.dispatches)\tfirst_pass=\(.first_pass_rate)\tesc=\(.escalation_rate)\tcache=\(.cache_hit_ratio)"'
        fi
        exit 0
        ;;
esac
