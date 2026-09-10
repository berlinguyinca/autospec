#!/usr/bin/env bash
# scripts/routing-ledger.sh — append-only JSONL outcome ledger for model routing.
#
# Records what each dispatch COST and how it turned out, so routing can be scored
# on measured effective cost instead of sticker price. Contract deliberately
# mirrors skills/autospec-shared/scripts/explore-ledger.sh (append-only, outcome
# enum, --stats, --show, --validate, --rebuild-shaped reader semantics) so there
# is one ledger idiom in the repo rather than two.
#
# Dispatch record schema (all keys required for --append):
#   {dispatch_id, ts, dispatch_kind, profile, model, harness, issue,
#    cell_ctx, cell_reasoning, input_tokens, output_tokens, cached_tokens,
#    wall_clock_ms, retries, escalated, outcome, reason}
#
#     dispatch_id     string, unique per dispatch attempt (NOT per issue — an
#                     issue can be dispatched many times)
#     dispatch_kind   implementer | lgtm-reviewer | explore-researcher |
#                     verify-voter | refine-lens | qa-sweep | secaudit-pass |
#                     growth-lens | spec-decompose | spec-research |
#                     spec-design | broad-audit | refine-scope
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
#     anchor_drops    OPTIONAL non-negative number (R8, tracker #3344): count
#                     of unanchored or non-verbatim claims dropped by
#                     local-dispatch.sh --verify-anchors on this dispatch.
#                     Absent when no anchor check ran. --stats totals it per
#                     (kind, profile, cell) so a profile that fabricates often
#                     becomes visible; --show prints it as drops=N.
#     outcome         pending | merged_clean | lgtm_first_pass | retried_ok |
#                     escalated | qa_failed | reverted | abandoned
#     stack           OPTIONAL. The detected stack-profile id this dispatch ran
#                     on (autospec-detect-stack-profile.sh). When present it
#                     must be a non-empty string; when absent, --append and
#                     --update-outcome normalize it to "unknown" so every row at
#                     rest carries the field. route-decide.sh reads this as the
#                     per-stack local-eligibility evidence (the stack gate).
#
# Telemetry fields (all OPTIONAL — the §25 per-dispatch telemetry extension):
#   role, model_version, hardware_fingerprint, runtime, quantization,
#   context_requested, context_reserved, context_used, concurrency_at_start,
#   queue_depth_at_start, prompt_tok_s, decode_tok_s, aggregate_decode_tok_s,
#   ttft_ms, retry_index, previous_model, review_outcome, tests_outcome,
#   merged, reverted
#
#     Each is the string "unknown" when the provider/harness did not report the
#     metric (unknown is preferable to a fabricated value — never 0) or a typed
#     value: a non-empty string, a non-negative number, or a boolean.
#     --append and --update-outcome normalize an absent field to "unknown" so
#     every row at rest carries the full §25 contract. A pre-extension legacy
#     row lacking the fields stays valid: --validate reads an absent key as
#     "unknown" (REQUIRED_KEYS is NOT extended).
#
# Event records (issue #3319) share this file. A Pi session's raw lifecycle and
# performance events are normalized by the Rust side
# (autospec_core::aar::normalize_event / normalize_jsonl, which emit the rows via
# to_ledger_lines) and appended here, so one ledger holds both the cost of each
# dispatch and what happened inside it, correlated by session_id and dispatch_id.
#   {record_type: "event", schema_version, seq, event, timestamp, session_id,
#    work_item_id, agent_role, harness, <metrics>}
#
#     event           session_start | model_request | tool_call | file_edit |
#                     test_run | compaction | failure | finish. Pi's own event
#                     names are mapped onto these before the row is written, so
#                     an unmapped name is an unknown producer and --validate
#                     fails closed on it.
#     seq             1-based position within the replayed session
#     timestamp, session_id, work_item_id, agent_role, harness
#                     mandatory non-empty strings. A row without identity cannot
#                     be correlated back to a work item, so it is a finding, not
#                     a row with a null column.
#     <metrics>       everything else: input_tokens, output_tokens,
#                     reasoning_tokens, cache_hit_tokens, cache_miss_tokens,
#                     ttft_ms, prefill_ms, prefill_tok_s, decode_tok_s, queue_ms,
#                     turn_ms, tool_ms, wall_ms, context_used_tokens,
#                     context_window_tokens, dispatch_id, model, tool, tool_ok,
#                     tests_total, tests_failed, repair_count, success,
#                     failure_category. Each is a JSON number, boolean or string,
#                     and the string "unknown" when Pi did not report it -- the
#                     same rule as the §25 fields above, and the reason a row is
#                     never written with 0 in place of a missing measurement.
#
# Event rows are NOT dispatch rows: --append writes them verbatim (the §25
# normalization above is dispatch-only), --validate routes them to their own
# contract, and --stats / --show / --rebuild skip them, because those group by
# dispatch_id over rows that have a profile and a cell.
#
# --rebuild reconstructs the latest record per dispatch_id from the existing
# ledger (§28: survive local loss where reconstructable from history) and
# writes it to <ledger>.rebuilt for the operator to diff/promote. It NEVER
# clobbers the live ledger: history is never rewritten.
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
#   routing-ledger.sh --validate [<file>]     (both record types)
#   routing-ledger.sh --rebuild
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
ALLOWED_KINDS="implementer lgtm-reviewer explore-researcher verify-voter refine-lens qa-sweep secaudit-pass growth-lens spec-decompose spec-research spec-design broad-audit refine-scope"
ALLOWED_CTX="32k 64k 120k"
ALLOWED_REASONING="shallow medium deep"

# Pi execution events (issue #3319): the canonical kind vocabulary after the
# Rust normalizer maps Pi's own event names, and the keys every event row must
# carry. Mirrors ALLOWED_EVENT_KINDS / EVENT_REQUIRED_KEYS in
# autospec_core::aar::pi_events -- keep the two lists in step (the same
# mirror discipline as learning/contracts.rs).
ALLOWED_EVENT_KINDS="session_start model_request tool_call file_edit test_run compaction failure finish"
EVENT_REQUIRED_KEYS="record_type schema_version seq event timestamp session_id work_item_id agent_role harness"

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
        --rebuild)  MODE="rebuild"; shift ;;
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
    # Two record types share one append-only file (issue #3319): dispatch rows
    # (this contract, record_type absent or "dispatch") and Pi execution event
    # rows (record_type "event", contract in _validate_event_object). Readers
    # that group by dispatch_id already skip event rows: _latest_records filters
    # has("dispatch_id"), so --stats / --show / --rebuild see dispatches only.
    if printf '%s' "$_obj" | jq -e '.record_type == "event"' >/dev/null 2>&1; then
        _validate_event_object "$_obj"
        return $?
    fi
    if printf '%s' "$_obj" | jq -e 'has("record_type") and (.record_type != "dispatch")' >/dev/null 2>&1; then
        printf 'unknown record_type: %s\n' "$(printf '%s' "$_obj" | jq -r '.record_type')"
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
    # stack is optional at rest (legacy rows predate it) but, when present, it
    # must be a non-empty string: a null or numeric stack would silently fail
    # every per-stack evidence query as if the row were from another stack.
    if printf '%s' "$_obj" | jq -e 'has("stack")' >/dev/null 2>&1; then
        if ! printf '%s' "$_obj" | jq -e '.stack | (type=="string") and (length>0)' >/dev/null 2>&1; then
            printf 'stack must be a non-empty string when present\n'
            return 1
        fi
    fi
    if ! _validate_counters "$_obj"; then
        return 1
    fi
    _validate_telemetry "$_obj"
}

# _validate_event_object <json> — the Pi execution-event half of the record
# contract (issue #3319). Written by the Rust normalizer
# (autospec_core::aar::normalize_event); validated here so one --validate pass
# checks every row of the file whoever appended it. Two rules mirror the Rust
# side exactly:
#
#   identity is mandatory. timestamp, session_id, work_item_id, agent_role and
#   harness must be non-empty strings: a row without identity cannot be
#   correlated back to a work item and silently poisons every aggregate.
#   the event name must be one of ALLOWED_EVENT_KINDS. Pi's own vocabulary is
#   mapped to these canonical names before the row is written, so an unmapped
#   name here means an unknown producer, not an unknown alias: fail closed.
#
# Every other key is a metric and must be a JSON number, boolean or string.
# "unknown" (the string) is how an unobserved metric is stored; null, an object
# or an array is a shape error, and 0 is a measurement, never a stand-in for
# "not measured".
_validate_event_object() {
    _obj="$1"
    for _k in $EVENT_REQUIRED_KEYS; do
        if ! printf '%s' "$_obj" | jq -e --arg k "$_k" 'has($k)' >/dev/null 2>&1; then
            printf 'event missing required key: %s\n' "$_k"
            return 1
        fi
    done
    for _k in timestamp session_id work_item_id agent_role harness; do
        if ! printf '%s' "$_obj" | jq -e --arg k "$_k" '.[$k] | (type=="string") and (length>0)' >/dev/null 2>&1; then
            printf 'event %s must be a non-empty string\n' "$_k"
            return 1
        fi
    done
    for _k in schema_version seq; do
        if ! printf '%s' "$_obj" | jq -e --arg k "$_k" '.[$k] | (type=="number") and (.>=1)' >/dev/null 2>&1; then
            printf 'event %s must be a number >= 1\n' "$_k"
            return 1
        fi
    done
    _ev="$(printf '%s' "$_obj" | jq -r '.event')"
    if ! _in_list "$_ev" "$ALLOWED_EVENT_KINDS"; then
        printf 'invalid event: %s\n' "$_ev"
        return 1
    fi
    _bad="$(printf '%s' "$_obj" | jq -r --argjson ok '["number","string","boolean"]' '
        . as $o
        | (keys_unsorted - ["record_type","schema_version","seq","event","timestamp",
              "session_id","work_item_id","agent_role","harness"])[]
        | . as $k
        | select(($ok | index($o[$k] | type)) == null)
        | "\($k)=\($o[$k] | type)"' 2>/dev/null | head -1)"
    if [ -n "$_bad" ]; then
        printf 'event metric %s has invalid type %s (want number, boolean, or the string unknown)\n' \
            "${_bad%%=*}" "${_bad##*=}"
        return 1
    fi
    return 0
}

# _normalize_unknowns <json-object-or-array> — re-emit each record with every
# optional field (stack + the 20 §25 telemetry fields) that is absent
# normalized to "unknown", so every row at rest carries the full contract.
# Absent means "the provider reported nothing": unknown, never 0. Input an
# array to normalize each element (emitted one compact record per line).
_normalize_unknowns() {
    _n_in="${1:-}"
    [ -n "$_n_in" ] || _n_in="$(cat)"
    printf '%s' "$_n_in" | jq -c '
        def norm: reduce (["stack","role","model_version","hardware_fingerprint",
                           "runtime","quantization","context_requested","context_reserved",
                           "context_used","concurrency_at_start","queue_depth_at_start",
                           "prompt_tok_s","decode_tok_s","aggregate_decode_tok_s",
                           "ttft_ms","retry_index","previous_model","review_outcome",
                           "tests_outcome","merged","reverted"][]) as $k
        (. ; if has($k) then . else . + {($k): "unknown"} end);
        if type == "array" then map(norm) | .[] else norm end'
}

# _validate_telemetry <json> — the §25 telemetry half of the record contract.
# All 20 fields are OPTIONAL: a pre-extension legacy row lacking them reads as
# "unknown" and stays valid. When present, each field must be the string
# "unknown" (the provider reported nothing) or a properly typed value — a
# non-empty string, a non-negative number, or a boolean. A fabricated or
# mistyped value fails closed: unknown is preferable to a fabricated metric.
_validate_telemetry() {
    _obj="$1"
    _bad="$(printf '%s' "$_obj" | jq -r '
        . as $o
        | ([ ["role","model_version","hardware_fingerprint","runtime","quantization",
             "previous_model","review_outcome","tests_outcome"][]
            | . as $k
            | select(($o|has($k)) and ((((($o[$k]|type)=="string") and ((($o[$k]=="unknown") or (($o[$k]|length)>0)))) | not)))
            | {k:$k, t:"\"unknown\" or a non-empty string"} ]
          + [ ["context_requested","context_reserved","context_used","concurrency_at_start",
              "queue_depth_at_start","prompt_tok_s","decode_tok_s","aggregate_decode_tok_s",
              "ttft_ms","retry_index"][]
            | . as $k
            | select(($o|has($k)) and ((((($o[$k]=="unknown") or (((($o[$k]|type)=="number") and ($o[$k]>=0))))) | not)))
            | {k:$k, t:"\"unknown\" or a non-negative number"} ]
          + [ ["merged","reverted"][]
            | . as $k
            | select(($o|has($k)) and ((($o[$k]=="unknown") or (($o[$k]|type)=="boolean")) | not))
            | {k:$k, t:"\"unknown\" or a boolean"} ]
        ) as $bad
        | if ($bad|length) > 0 then $bad[0] | "\(.k) / \(.t)" else empty end')"
    if [ -n "$_bad" ]; then
        _k="${_bad%% /*}"
        _t="${_bad#* / }"
        printf 'telemetry field %s must be %s\n' "$_k" "$_t"
        return 1
    fi
    return 0
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
    # anchor_drops (R8) is optional: absent means no verbatim-anchor check ran.
    # Present, it must be a non-negative number — it is the fabrication signal
    # the ledger exists to make visible, and a string would poison the total.
    if printf '%s' "$_obj" | jq -e 'has("anchor_drops")' >/dev/null 2>&1; then
        if ! printf '%s' "$_obj" | jq -e '(.anchor_drops|type=="number") and (.anchor_drops>=0)' >/dev/null 2>&1; then
            printf 'anchor_drops must be a non-negative number\n'
            return 1
        fi
    fi
    return 0
}

# Latest line per dispatch_id, preserving first-seen order.
_latest_records() {
    if [ ! -f "$LEDGER" ]; then
        printf '[]'
        return 0
    fi
    jq -s 'map(select(type=="object" and has("dispatch_id")
          and ((.record_type // "dispatch") == "dispatch")))
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
        # Normalize absent optional fields (stack + §25 telemetry) to
        # "unknown" so every row at rest carries the full contract. Pi event
        # rows (issue #3319) carry their own contract and are appended as
        # compacted-but-otherwise-verbatim JSON: the dispatch telemetry keys are
        # not part of an event, and injecting them would make every event row
        # look like a dispatch row that measured nothing.
        if printf '%s' "$ARG1" | jq -e '.record_type == "event"' >/dev/null 2>&1; then
            printf '%s\n' "$(printf '%s' "$ARG1" | jq -c '.')" >> "$LEDGER"
        else
            printf '%s\n' "$(_normalize_unknowns "$ARG1")" >> "$LEDGER"
        fi
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
        # A pre-stack legacy row is normalized the same way --append normalizes,
        # so the "every row carries stack" invariant survives outcome updates too.
        printf '%s' "$_prev" | jq -c \
            --arg oc "$ARG2" --arg rs "$ARG3" \
            --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            '.outcome=$oc | .ts=$ts | (if $rs != "" then .reason=$rs else . end)' \
            | _normalize_unknowns >> "$LEDGER"
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
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\t\(.outcome)\tretries=\(.retries)\tms=\(.wall_clock_ms)\tdrops=\(.anchor_drops // 0)"'
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
                anchor_drops:  (map(.anchor_drops // 0) | add // 0),
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
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\tn=\(.dispatches)\tfirst_pass=\(.first_pass_rate)\tesc=\(.escalation_rate)\tcache=\(.cache_hit_ratio)\tdrops=\(.anchor_drops)"'
        fi
        exit 0
        ;;
    rebuild)
        # §28: survive local loss where reconstructable from history — but
        # history is never rewritten: the rebuild writes <ledger>.rebuilt for
        # the operator to diff/promote, and never clobbers the live ledger.
        if [ ! -f "$LEDGER" ]; then
            _die "no ledger at $LEDGER; nothing to reconstruct"
        fi
        _out="${LEDGER}.rebuilt"
        if [ ! -d "$(dirname "$_out")" ]; then
            mkdir -p "$(dirname "$_out")"
        fi
        _normalize_unknowns "$(_latest_records)" > "$_out"
        printf 'rebuilt: %s latest record(s) from %s -> %s\n' \
            "$(grep -c . "$_out" 2>/dev/null || true)" "$LEDGER" "$_out"
        ;;
esac
