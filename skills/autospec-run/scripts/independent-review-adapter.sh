#!/usr/bin/env bash
# Fail-closed adapter between skill orchestration and an independent reviewer.

set -u
umask 077

usage() {
    printf '%s\n' \
        'Usage: independent-review-adapter.sh prepare --repo R --issue N --pr N --commit OID --risk RISK --implementer-provider P --reviewer-provider P --reviewer-reasoning standard|high --foreground-available true|false --request-out FILE' \
        '       independent-review-adapter.sh validate --request FILE --verdict FILE'
}

die() {
    printf 'independent-review-adapter: %s\n' "$*" >&2
    exit 2
}

require_value() {
    [ -n "$2" ] || die "missing $1"
}

json_has_unique_keys() {
    command -v python3 >/dev/null 2>&1 || return 1
    python3 - "$1" <<'PY' >/dev/null 2>&1
import json
import sys

def reject_duplicates(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate key: {key}")
        obj[key] = value
    return obj

with open(sys.argv[1], encoding="utf-8") as handle:
    json.load(handle, object_pairs_hook=reject_duplicates)
PY
}

requeue_review() {
    blocker="$1"
    reason="code_health:${blocker} risk=${RISK} pr=${PR}"
    gh issue comment "$ISSUE" --repo "$REPO" \
        --body "$reason; autonomous merge is forbidden; requeued for an independent reviewer" \
        >/dev/null || die 'could not record review blocker'
    gh issue edit "$ISSUE" --repo "$REPO" \
        --remove-label in-progress-by-bot --add-label auto-implement \
        >/dev/null || die 'could not requeue blocked review'
    jq -nc --arg code "$blocker" --arg risk "$RISK" --arg pr "$PR" \
        '{schema:1,outcome:"requeue",blocker:{code:$code,risk:$risk,pr:$pr}}'
    exit 75
}

write_request() {
    integration_required=false
    [ "$RISK" = integration ] || [ "$RISK" = critical ] && integration_required=true
    provider_diversified=false
    [ "$IMPLEMENTER_PROVIDER" != "$REVIEWER_PROVIDER" ] && provider_diversified=true
    request_tmp="${REQUEST_OUT}.tmp.$$"
    jq -n \
        --arg commit "$COMMIT" \
        --arg risk "$RISK" \
        --arg implementer "$IMPLEMENTER_PROVIDER" \
        --arg reviewer "$REVIEWER_PROVIDER" \
        --arg reasoning "$REVIEWER_REASONING" \
        --argjson integration "$integration_required" \
        --argjson diversified "$provider_diversified" \
        '{
            schema: 1,
            commit: $commit,
            policy: {
                risk: $risk,
                implementer_provider: $implementer,
                reviewer_provider: $reviewer,
                reviewer_reasoning: $reasoning,
                provider_diversified: $diversified,
                require_integration_paths: $integration
            },
            reviewer: {independent: true, read_only: true, foreground: true},
            verdict_contract: {
                schema: 1,
                commit: $commit,
                verdict: ["lgtm", "blocked"],
                surfaces_examined: "nonempty-string-array",
                tests_examined: "nonempty-string-array",
                integration_paths_checked: (if $integration then "required-nonempty-string-array" else "string-array" end),
                blocking_findings: "string-array"
            }
        }' > "$request_tmp" || return 1
    mv "$request_tmp" "$REQUEST_OUT"
}

validate_prepare_inputs() {
    for entry in \
        "REPO:$REPO" "ISSUE:$ISSUE" "PR:$PR" "COMMIT:$COMMIT" \
        "RISK:$RISK" "IMPLEMENTER_PROVIDER:$IMPLEMENTER_PROVIDER" \
        "REVIEWER_PROVIDER:$REVIEWER_PROVIDER" \
        "REVIEWER_REASONING:$REVIEWER_REASONING" \
        "FOREGROUND_AVAILABLE:$FOREGROUND_AVAILABLE" "REQUEST_OUT:$REQUEST_OUT"
    do
        require_value "${entry%%:*}" "${entry#*:}"
    done
    printf '%s' "$COMMIT" | grep -Eq '^[0-9a-fA-F]{40}$' || die 'commit must be 40 hex characters'
    case "$RISK" in normal|high|integration|critical) ;; *) die 'risk is invalid' ;; esac
    case "$REVIEWER_REASONING" in standard|high) ;; *) die 'reviewer reasoning is invalid' ;; esac
    case "$FOREGROUND_AVAILABLE" in true|false) ;; *) die 'foreground availability must be true or false' ;; esac
    if [ "$RISK" = normal ] && [ "$REVIEWER_REASONING" != standard ]; then
        die 'normal review requires standard reasoning'
    fi
    if [ "$RISK" != normal ] && [ "$REVIEWER_REASONING" != high ]; then
        die 'non-normal review requires high reasoning'
    fi
}

prepare_review() {
    REPO=""
    ISSUE=""
    PR=""
    COMMIT=""
    RISK=""
    IMPLEMENTER_PROVIDER=""
    REVIEWER_PROVIDER=""
    REVIEWER_REASONING=""
    FOREGROUND_AVAILABLE=""
    REQUEST_OUT=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) REPO="${2:-}"; shift 2 ;;
            --issue) ISSUE="${2:-}"; shift 2 ;;
            --pr) PR="${2:-}"; shift 2 ;;
            --commit) COMMIT="${2:-}"; shift 2 ;;
            --risk) RISK="${2:-}"; shift 2 ;;
            --implementer-provider) IMPLEMENTER_PROVIDER="${2:-}"; shift 2 ;;
            --reviewer-provider) REVIEWER_PROVIDER="${2:-}"; shift 2 ;;
            --reviewer-reasoning) REVIEWER_REASONING="${2:-}"; shift 2 ;;
            --foreground-available) FOREGROUND_AVAILABLE="${2:-}"; shift 2 ;;
            --request-out) REQUEST_OUT="${2:-}"; shift 2 ;;
            *) die "unknown argument: $1" ;;
        esac
    done
    validate_prepare_inputs
    write_request || die 'could not write structured review request'
    [ "$FOREGROUND_AVAILABLE" = true ] || requeue_review independent_review_unavailable
    if [ "$RISK" = critical ] && [ "$IMPLEMENTER_PROVIDER" = "$REVIEWER_PROVIDER" ]; then
        requeue_review provider_diversity_required
    fi
    jq -nc --arg request "$REQUEST_OUT" '{schema:1,outcome:"prepared",request:$request}'
}

validate_review() {
    request=""
    verdict=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --request) request="${2:-}"; shift 2 ;;
            --verdict) verdict="${2:-}"; shift 2 ;;
            *) die "unknown argument: $1" ;;
        esac
    done
    require_value request "$request"
    require_value verdict "$verdict"
    [ -f "$request" ] || die 'review request does not exist'
    [ -f "$verdict" ] || die 'review verdict does not exist'
    if ! json_has_unique_keys "$request" || ! json_has_unique_keys "$verdict"; then
        jq -nc '{schema:1,outcome:"block",blocker:{code:"structured_review_invalid"}}'
        return 1
    fi
    if jq -e --slurpfile request "$request" '
        def nonempty_strings:
            type == "array" and length > 0 and
            all(.[]; type == "string" and test("[^[:space:]]"));
        def strings:
            type == "array" and all(.[]; type == "string" and test("[^[:space:]]"));
        type == "object" and
        keys == ["blocking_findings", "commit", "integration_paths_checked", "schema", "surfaces_examined", "tests_examined", "verdict"] and
        .schema == 1 and
        .commit == $request[0].commit and
        .verdict == "lgtm" and
        (.surfaces_examined | nonempty_strings) and
        (.tests_examined | nonempty_strings) and
        (.integration_paths_checked | strings) and
        (($request[0].policy.require_integration_paths | not) or (.integration_paths_checked | length > 0)) and
        (.blocking_findings | strings) and
        (.blocking_findings | length == 0)
    ' "$verdict" >/dev/null 2>&1; then
        printf '%s\n' LGTM
        return 0
    fi
    jq -nc '{schema:1,outcome:"block",blocker:{code:"structured_review_invalid"}}'
    return 1
}

command_name="${1:-}"
[ -n "$command_name" ] || { usage >&2; exit 2; }
shift
case "$command_name" in
    prepare) prepare_review "$@" ;;
    validate) validate_review "$@" ;;
    -h|--help) usage ;;
    *) usage >&2; exit 2 ;;
esac
