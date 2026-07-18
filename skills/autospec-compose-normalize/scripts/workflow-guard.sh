#!/usr/bin/env bash
# Deterministic lookup, refusal, and strict claim helpers for Compose migration.

set -euo pipefail

PROG=autospec-compose-normalize-guard.sh
CLAIM_GUARD=${AUTOSPEC_CLAIM_GUARD_SH:-${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/claim-guard.sh}

usage() {
    cat <<'EOF'
Usage:
  autospec-compose-normalize-guard.sh lookup SHA256
  autospec-compose-normalize-guard.sh search-query SHA256
  autospec-compose-normalize-guard.sh select SHA256 ISSUE_JSON PR_JSON
  autospec-compose-normalize-guard.sh direct-refuse SHA256 [ISSUE_JSON PR_JSON]
  autospec-compose-normalize-guard.sh new-session-token
  autospec-compose-normalize-guard.sh claim acquire|verify|refresh|release SESSION PATH...
EOF
}

die() {
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit 2
}

validate_fingerprint() {
    [[ ${1:-} =~ ^[0-9a-f]{64}$ ]] || die "fingerprint must be 64 lowercase hexadecimal characters"
}

search_query() {
    printf '%s in:body\n' "$1"
}

filter_json() {
    local kind=$1 fingerprint=$2 json=$3 marker
    marker="<!-- autospec-compose-fingerprint: $fingerprint -->"
    printf '%s\n' "$json" | jq -r --arg marker "$marker" --arg kind "$kind" '
      .[] | select((.body // "") | contains($marker)) |
      [$kind, (if $kind == "pr" and .mergedAt != null then "MERGED" else .state end),
       (.number | tostring), .url] | @tsv
    '
}

lookup_kind() {
    local kind=$1 fingerprint=$2 query json
    query=$(search_query "$fingerprint")
    case "$kind" in
        issue)
            json=$(gh issue list --state all --limit 100 \
                --search "$query" --json number,state,url,body)
            ;;
        pr)
            json=$(gh pr list --state all --limit 100 \
                --search "$query" --json number,state,mergedAt,url,body)
            ;;
        *) die "unknown lookup kind: $kind" ;;
    esac
    filter_json "$kind" "$fingerprint" "$json"
}

lookup() {
    local fingerprint=$1
    lookup_kind issue "$fingerprint"
    lookup_kind pr "$fingerprint"
}

select_files() {
    local fingerprint=$1 issue_file=$2 pr_file=$3
    [[ -f $issue_file && -f $pr_file ]] || die "select requires readable issue and pull-request JSON files"
    filter_json issue "$fingerprint" "$(<"$issue_file")"
    filter_json pr "$fingerprint" "$(<"$pr_file")"
}

direct_refuse() {
    local fingerprint=$1 matches
    shift
    if [[ $# -eq 2 ]]; then
        matches=$(select_files "$fingerprint" "$1" "$2")
    elif [[ $# -eq 0 ]]; then
        matches=$(lookup "$fingerprint")
    else
        die "direct-refuse accepts either SHA256 or SHA256 ISSUE_JSON PR_JSON"
    fi
    if [[ -n $matches ]]; then
        printf 'Compose isolation migration already exists:\n%s\n' "$matches"
    else
        printf 'No issue or pull request exists for Compose fingerprint %s.\n' "$fingerprint"
    fi
    printf '%s\n' 'Runtime provisioning was not started from this unmanaged session.'
    exit 3
}

new_session_token() {
    python3 -c 'import secrets; print("compose-normalize-" + secrets.token_hex(24))'
}

claim_key() {
    local path=${1#./} rest name
    path=${path%/}
    case "$path" in
        skills/*)
            rest=${path#skills/}; name=${rest%%/*}; printf 'skill:%s' "$name" ;;
        tests/fixtures/skill-goldens/*)
            rest=${path#tests/fixtures/skill-goldens/}; name=${rest%%.*}; printf 'skill:%s' "$name" ;;
        *) printf 'path:%s' "$path" ;;
    esac
}

run_claim_guard() {
    local token=$1
    shift
    env AUTOSPEC_CLAIM_GUARD=strict AUTOSPEC_SESSION_ID="$token" \
        bash "$CLAIM_GUARD" "$@"
}

verify_claims() {
    local token=$1 status target key owner
    shift
    run_claim_guard "$token" assert "$@"
    status=$(run_claim_guard "$token" status)
    owner="owner=$token"
    for target in "$@"; do
        key=$(claim_key "$target")
        if ! awk -F '\t' -v key="$key" -v owner="$owner" \
            '$1 == key && $2 == owner { found=1 } END { exit !found }' <<<"$status"; then
            printf 'code_health:compose_claim_not_persisted key=%s owner_session=%s\n' \
                "$key" "$token" >&2
            return 1
        fi
    done
}

verify_released() {
    local token=$1 status target key owner
    shift
    status=$(run_claim_guard "$token" status)
    owner="owner=$token"
    for target in "$@"; do
        key=$(claim_key "$target")
        if awk -F '\t' -v key="$key" -v owner="$owner" \
            '$1 == key && $2 == owner { found=1 } END { exit !found }' <<<"$status"; then
            printf 'code_health:compose_claim_not_released key=%s\n' "$key" >&2
            return 1
        fi
    done
}

claim() {
    local operation=${1:-} token=${2:-}
    shift 2 || die "claim requires an operation and stable session token"
    [[ -n $token && $token != *$'\n'* && $token != *$'\t'* ]] || die "invalid claim session token"
    [[ $# -ge 1 ]] || die "claim $operation requires at least one path"
    case "$operation" in
        acquire) run_claim_guard "$token" acquire "$@"; verify_claims "$token" "$@" ;;
        verify) verify_claims "$token" "$@" ;;
        refresh) run_claim_guard "$token" refresh; verify_claims "$token" "$@" ;;
        release) run_claim_guard "$token" release "$@"; verify_released "$token" "$@" ;;
        *) die "unknown claim operation: $operation" ;;
    esac
}

main() {
    local command=${1:-}
    shift || true
    case "$command" in
        lookup) [[ $# -eq 1 ]] || die "lookup requires SHA256"; validate_fingerprint "$1"; lookup "$1" ;;
        search-query) [[ $# -eq 1 ]] || die "search-query requires SHA256"; validate_fingerprint "$1"; search_query "$1" ;;
        select) [[ $# -eq 3 ]] || die "select requires SHA256 ISSUE_JSON PR_JSON"; validate_fingerprint "$1"; select_files "$@" ;;
        direct-refuse) [[ $# -eq 1 || $# -eq 3 ]] || die "direct-refuse requires SHA256 [ISSUE_JSON PR_JSON]"; validate_fingerprint "$1"; direct_refuse "$@" ;;
        new-session-token) [[ $# -eq 0 ]] || die "new-session-token accepts no arguments"; new_session_token ;;
        claim) claim "$@" ;;
        -h|--help) usage ;;
        *) usage >&2; exit 2 ;;
    esac
}

main "$@"
