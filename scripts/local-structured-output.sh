#!/usr/bin/env bash
# scripts/local-structured-output.sh — R7 structured-output gate (issue #3350).
#
# Guardrail R7: a schema validator is a *precondition* for local
# structured-output routing. Local models' JSON/YAML conformance degrades
# faster than their prose, and an unvalidated structured contract cannot be
# distinguished from a good one after the fact. So:
#
#   * This script is the single source of truth for the declared
#     kind -> output-schema -> validator-command mapping.
#   * `route-decide.sh` consults `--local-eligible <kind>` before it offers a
#     local profile: a kind with a declared schema but NO registered validator
#     is denied local (default deny) and falls through to the baseline.
#   * Where a validator IS registered, the local dispatch is wrapped with
#     `--dispatch`: every attempt is schema-validated, the validator's
#     findings are fed back to the local model as directives, the retry is
#     bounded (max 5 by default), and exhaustion escalates UP to the cloud
#     fallback command — never another local attempt. The retry count and
#     final outcome land on a routing-ledger row.
#
# Validator contract: the validator command receives ONE positional argument
# (the dispatch output file). Exit 0 = the output satisfies the schema;
# non-zero = findings printed to stderr.
#
# Usage:
#   local-structured-output.sh --kinds
#   local-structured-output.sh --validator <kind>
#   local-structured-output.sh --local-eligible <kind>
#   local-structured-output.sh --dispatch <kind>
#                              --local <cmd> [args...]
#                              --fallback <cmd> [args...]
#                              [--validator-cmd <cmd>] [--max-retries N]
#                              [--record '<json>'] [--ledger <path>]
#
# Exit codes:
#   --kinds / --local-eligible:
#     0  eligible (no declared schema, or a validator is registered)
#     1  a declared schema exists but no validator is registered
#   --validator:
#     0  validator registered (command printed on stdout)
#     1  kind declares a schema but no validator is registered
#     3  kind has no declared schema
#   --dispatch:
#     0  local output validated (after <= N retries) or escalation succeeded
#     1  bad arguments
#     4  no resolvable validator for the kind — refuse before any dispatch
#     otherwise: the fallback command's own exit status after escalation
#
# Environment:
#   (none — all configuration is explicit flags; see docs/CONFIG_REFERENCE.md)

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

# ── declared mapping: dispatch kind -> output schema -> validator command ────
# One row per declared structured kind, pipe-separated:
#   kind|schema|validator-command
# validator-command is EMPTY while no validator is registered: the kind is
# then not local-eligible (the gate denies local, the dispatcher refuses).
# Kinds absent from this table declare no schema: the gate does not apply to
# them and routing is unchanged. Writing the validators for the two declared
# kinds is out of scope here (filed separately); until they land, these kinds
# are baseline-only even though they are on the overridable allowlist.
STRUCTURED_KIND_MAP="
explore-researcher|findings-json|
qa-sweep|findings-json|
"

# Default retry budget: at most 5 retries AFTER the first local attempt, so a
# local dispatch is executed at most 6 times before escalation to cloud.
DEFAULT_MAX_RETRIES=5

lso_usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }
lso_die() { printf 'local-structured-output: %s\n' "$1" >&2; exit "${2:-1}"; }

# lso_lookup <kind> — set LSO_SCHEMA / LSO_VALIDATOR for a declared kind.
# Returns 0 when the kind is declared, 1 when it has no declared schema.
lso_lookup() {
    local line k s v
    LSO_SCHEMA=""
    LSO_VALIDATOR=""
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        IFS='|' read -r k s v <<<"$line"
        if [ "$k" = "$1" ]; then
            LSO_SCHEMA="$s"
            LSO_VALIDATOR="${v:-}"
            return 0
        fi
    done <<<"$STRUCTURED_KIND_MAP"
    return 1
}

lso_kinds() {
    printf '%s\n' "$STRUCTURED_KIND_MAP" \
        | awk -F'|' 'NF >= 2 && $1 != "" {print $1}' \
        | sort
}

# lso_validator_of <kind> — print the registered validator command.
lso_validator_of() {
    local kind="${1:-}"
    [ -n "$kind" ] || lso_die 'a kind is required'
    if ! lso_lookup "$kind"; then
        printf 'local-structured-output: kind %s declares no schema; no validator applies (exit 3)\n' "$kind" >&2
        exit 3
    fi
    if [ -z "$LSO_VALIDATOR" ]; then
        printf 'local-structured-output: kind %s (schema %s) has no registered validator (exit 1)\n' "$kind" "$LSO_SCHEMA" >&2
        exit 1
    fi
    printf '%s\n' "$LSO_VALIDATOR"
}

# lso_local_eligible <kind> — the gate `route-decide.sh` consults.
lso_local_eligible() {
    local kind="${1:-}"
    [ -n "$kind" ] || lso_die 'a kind is required'
    if ! lso_lookup "$kind"; then
        return 0  # no declared schema: the gate does not apply
    fi
    if [ -n "$LSO_VALIDATOR" ]; then
        return 0  # validator registered: the --dispatch wrapper enforces R7
    fi
    printf 'local-structured-output: kind %s (schema %s) has no registered validator; not local-eligible\n' \
        "$kind" "$LSO_SCHEMA" >&2
    return 1
}

# lso_validate <validator-cmd> <output-file> <stderr-file>
# Runs the validator; returns its exit status, findings left in <stderr-file>.
# $1 is a command string declared by this repository (the mapping table or a
# --validator-cmd override), not caller input: word-splitting it is deliberate.
lso_validate() {
    # shellcheck disable=SC2086
    $1 "$2" >/dev/null 2>"$3"
}

# lso_record <record-json> <retries> <outcome> <escalated-0|1> <reason> <ledger>
# Appends the dispatch outcome to the routing ledger via routing-ledger.sh.
lso_record() {
    local record="$1" retries="$2" outcome="$3" escalated="$4" reason="$5" ledger="$6"
    local ts updated esc_json
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ "$escalated" = "1" ]; then esc_json="true"; else esc_json="false"; fi
    updated="$(jq -c \
        --argjson retries "$retries" \
        --arg outcome "$outcome" \
        --argjson escalated "$esc_json" \
        --arg reason "$reason" \
        --arg ts "$ts" \
        '.retries = $retries | .outcome = $outcome | .escalated = $escalated | .reason = $reason | .ts = $ts' \
        <<<"$record")" || lso_die 'failed to update the ledger record'
    if [ -n "$ledger" ]; then
        bash "$SCRIPT_DIR/routing-ledger.sh" --ledger "$ledger" --append "$updated" \
            || lso_die 'failed to append the ledger row'
    else
        bash "$SCRIPT_DIR/routing-ledger.sh" --append "$updated" \
            || lso_die 'failed to append the ledger row'
    fi
}

# lso_dispatch — the bounded-retry local dispatch wrapper.
lso_dispatch() {
    local kind="$1"
    local validator="$2"
    local max_retries="$3"
    local record="$4"
    local ledger="$5"
    [ $# -ge 6 ] || lso_die '--dispatch requires --local <cmd> [args...]'
    local -a local_cmd=("${@:6}")

    [ -n "$kind" ] || lso_die '--dispatch requires --kind'
    [ ${#FALLBACK_CMD[@]} -gt 0 ] || lso_die '--dispatch requires --fallback <cmd> [args...]'
    case "$max_retries" in
        ''|*[!0-9]*) lso_die "--max-retries must be a non-negative integer: $max_retries" ;;
    esac

    # Fail closed before spending a single local attempt.
    if ! lso_lookup "$kind"; then
        printf 'local-structured-output: kind %s declares no schema; dispatch it plainly, not through the R7 wrapper (exit 4)\n' "$kind" >&2
        exit 4
    fi
    # A --validator-cmd override wins; otherwise the registered validator from
    # the declared mapping is the only one that may gate this dispatch.
    if [ -z "$validator" ]; then
        validator="$LSO_VALIDATOR"
    fi
    if [ -z "$validator" ]; then
        printf 'local-structured-output: kind %s (schema %s) has no registered validator and no --validator-cmd override; refusing an unvalidated local dispatch (exit 4)\n' \
            "$kind" "$LSO_SCHEMA" >&2
        exit 4
    fi

    local out_file err_file val_err fallback_out
    out_file="$(mktemp "${TMPDIR:-/tmp}/lso-out.XXXXXX")" || lso_die 'mktemp failed'
    err_file="$(mktemp "${TMPDIR:-/tmp}/lso-err.XXXXXX")" || lso_die 'mktemp failed'
    val_err="$(mktemp "${TMPDIR:-/tmp}/lso-valerr.XXXXXX")" || lso_die 'mktemp failed'
    fallback_out="$(mktemp "${TMPDIR:-/tmp}/lso-fb.XXXXXX")" || lso_die 'mktemp failed'
    trap 'rm -f "$out_file" "$err_file" "$val_err" "$fallback_out"' EXIT INT TERM

    local directives="" retries=0 attempt=1 validated=0
    while :; do
        # Attempt N: run the local command; from the second attempt on, the
        # previous validator findings ride along as directives.
        : >"$out_file"
        : >"$err_file"
        if [ -n "$directives" ]; then
            env AUTOSPEC_VALIDATION_DIRECTIVES="$directives" "${local_cmd[@]}" >"$out_file" 2>"$err_file"
        else
            env -u AUTOSPEC_VALIDATION_DIRECTIVES "${local_cmd[@]}" >"$out_file" 2>"$err_file"
        fi
        local lrc=$?

        if [ "$lrc" -ne 0 ]; then
            directives="local dispatch exited $lrc without a complete output"
            if [ -s "$err_file" ]; then
                directives="$directives: $(tail -n 3 "$err_file" | tr '\n' ' ')"
            fi
            printf 'local-structured-output: local attempt %s failed (exit %s); %s\n' \
                "$attempt" "$lrc" \
                "$([ "$retries" -ge "$max_retries" ] && printf 'escalating to cloud' || printf '%s retried attempt(s) remain' "$((max_retries - retries))")" \
                >&2
        elif lso_validate "$validator" "$out_file" "$val_err"; then
            validated=1
            break
        else
            directives="$(cat "$val_err")"
            [ -n "$directives" ] || directives="schema validation failed with no findings"
            printf 'local-structured-output: local attempt %s failed schema validation; %s\n' \
                "$attempt" \
                "$([ "$retries" -ge "$max_retries" ] && printf 'escalating to cloud' || printf '%s retried attempt(s) remain' "$((max_retries - retries))")" \
                >&2
        fi

        if [ "$retries" -ge "$max_retries" ]; then
            break  # budget exhausted: never another local attempt
        fi
        retries=$((retries + 1))
        attempt=$((attempt + 1))
    done

    local outcome escalated reason fbrc=0
    if [ "$validated" -eq 1 ]; then
        escalated=0
        if [ "$retries" -eq 0 ]; then
            outcome="lgtm_first_pass"
            reason=""
        else
            outcome="retried_ok"
            reason="schema validation passed after $retries retried attempt(s)"
        fi
        cat "$out_file"
    else
        # Escalate UP to the cloud fallback; it runs exactly once.
        printf 'local-structured-output: local retries exhausted (%s); escalating to cloud fallback\n' "$retries" >&2
        "${FALLBACK_CMD[@]}" >"$fallback_out"
        fbrc=$?
        escalated=1
        outcome="escalated"
        reason="local schema validation failed on all $((retries + 1)) attempts; escalated to cloud"
        cat "$fallback_out"
    fi

    if [ -n "$record" ]; then
        lso_record "$record" "$retries" "$outcome" "$escalated" "$reason" "$ledger"
    fi
    exit "$fbrc"
}

# ── argument parsing ─────────────────────────────────────────────────────────

MODE=""
KIND=""
VALIDATOR_CMD=""
MAX_RETRIES="$DEFAULT_MAX_RETRIES"
RECORD=""
LEDGER=""
LOCAL_CMD=()
FALLBACK_CMD=()

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) lso_usage; exit 0 ;;
        --kinds)
            [ "$MODE" = "" ] || lso_die 'choose exactly one mode'
            MODE="kinds"
            shift
            ;;
        --local-eligible)
            [ "$MODE" = "" ] || lso_die 'choose exactly one mode'
            MODE="local-eligible"
            KIND="${2:-}"
            [ $# -ge 2 ] || lso_die '--local-eligible requires a kind'
            shift 2
            ;;
        --validator)
            [ "$MODE" = "" ] || lso_die 'choose exactly one mode'
            MODE="validator"
            KIND="${2:-}"
            [ $# -ge 2 ] || lso_die '--validator requires a kind'
            shift 2
            ;;
        --dispatch)
            [ "$MODE" = "" ] || lso_die 'choose exactly one mode'
            MODE="dispatch"
            KIND="${2:-}"
            [ $# -ge 2 ] || lso_die '--dispatch requires a kind'
            shift 2
            ;;
        --local)
            shift
            LOCAL_CMD=()
            while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do
                LOCAL_CMD+=("$1")
                shift
            done
            ;;
        --fallback)
            shift
            FALLBACK_CMD=()
            while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do
                FALLBACK_CMD+=("$1")
                shift
            done
            ;;
        --validator-cmd)
            VALIDATOR_CMD="${2:-}"
            [ $# -ge 2 ] || lso_die '--validator-cmd requires a command'
            shift 2
            ;;
        --max-retries)
            MAX_RETRIES="${2:-}"
            [ $# -ge 2 ] || lso_die '--max-retries requires a value'
            shift 2
            ;;
        --record)
            RECORD="${2:-}"
            [ $# -ge 2 ] || lso_die '--record requires a JSON record'
            shift 2
            ;;
        --ledger)
            LEDGER="${2:-}"
            [ $# -ge 2 ] || lso_die '--ledger requires a path'
            shift 2
            ;;
        *) lso_die "unknown option: $1" ;;
    esac
done

case "$MODE" in
    kinds) lso_kinds ;;
    local-eligible) lso_local_eligible "$KIND" ;;
    validator) lso_validator_of "$KIND" ;;
    dispatch)
        if [ ${#LOCAL_CMD[@]} -gt 0 ]; then
            lso_dispatch "$KIND" "$VALIDATOR_CMD" "$MAX_RETRIES" "$RECORD" "$LEDGER" "${LOCAL_CMD[@]}"
        else
            lso_dispatch "$KIND" "$VALIDATOR_CMD" "$MAX_RETRIES" "$RECORD" "$LEDGER"
        fi
        ;;
    *) lso_usage >&2; exit 1 ;;
esac
