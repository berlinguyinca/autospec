#!/usr/bin/env bash
# scripts/issue-snapshot.sh — single-fetch issue field snapshot (D5 extension).
#
# The monitor loop historically issued four separate `gh issue view` calls per
# issue (body, title, url, labels) — four API round-trips and four tool results
# in the orchestrator context. This helper collapses them into ONE
# `gh issue view --json body,title,url,labels` call and caches the JSON per
# issue so later steps reuse the file instead of re-fetching.
#
# Usage:
#   issue-snapshot.sh get <ISSUE> [--refresh] [--dir DIR]
#       Fetch (or reuse) the snapshot for <ISSUE> and print its path.
#       Cache-first: an existing non-empty snapshot is reused without network
#       unless --refresh is given. The fetch is atomic (temp + mv) so a failed
#       refresh never clobbers a good snapshot.
#   issue-snapshot.sh -h | --help
#
# Snapshot path: <dir>/autospec-issue-<ISSUE>.json where <dir> is, in order:
#   1. --dir DIR
#   2. $AUTOSPEC_SNAPSHOT_DIR
#   3. /tmp
#
# Exit codes: 0 success (path on stdout), 1 fetch failure, 2 usage error.
#
# Conventions: set -eu; if/then/fi for one-sided conditionals; no RETURN traps
# (repo bash 3.2 gotchas). Compatible with bash 3.2+.

set -eu

PROG="issue-snapshot.sh"

usage() {
    cat <<'EOF'
Usage:
  issue-snapshot.sh get <ISSUE> [--refresh] [--dir DIR]
  issue-snapshot.sh -h | --help

get: single `gh issue view --json body,title,url,labels` fetch, cached per
     issue; prints the snapshot path. --refresh forces a fresh fetch.
     Snapshot path: <dir>/autospec-issue-<ISSUE>.json
     <dir> = --dir DIR > $AUTOSPEC_SNAPSHOT_DIR > /tmp

Exit codes: 0 ok, 1 fetch failure, 2 usage error.
EOF
}

die() {  # die <code> <message...>
    local code="$1"
    shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

validate_issue() {  # validate_issue <ISSUE>
    case "${1:-}" in
        ''|*[!0-9]*) die 2 "issue number must be a positive integer (got: '${1:-}')" ;;
    esac
}

snapshot_path() {  # snapshot_path <dir> <ISSUE>
    printf '%s/autospec-issue-%s.json\n' "$1" "$2"
}

cmd_get() {  # cmd_get <ISSUE> <refresh:0|1> <dir>
    local issue="$1" refresh="$2" dir="$3"
    local path tmp json
    path="$(snapshot_path "$dir" "$issue")"

    if [ "$refresh" -eq 0 ] && [ -s "$path" ]; then
        printf '%s\n' "$path"
        return 0
    fi

    mkdir -p "$dir"
    tmp="$(mktemp "${path}.XXXXXX")"
    if ! json="$(gh issue view "$issue" --json body,title,url,labels 2>/dev/null)"; then
        rm -f "$tmp"
        die 1 "gh issue view failed for issue #$issue (existing snapshot, if any, left intact)"
    fi
    if ! printf '%s\n' "$json" | jq -e '.body != null or .title != null' >/dev/null 2>&1; then
        rm -f "$tmp"
        die 1 "unexpected gh issue view payload for issue #$issue"
    fi
    printf '%s\n' "$json" > "$tmp"
    mv "$tmp" "$path"
    printf '%s\n' "$path"
}

main() {
    local cmd="${1:-}"
    if [ -z "$cmd" ]; then
        usage >&2
        exit 2
    fi
    case "$cmd" in
        -h|--help)
            usage
            exit 0
            ;;
        get) ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
    shift

    local refresh=0 dir="" issue=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --refresh)
                refresh=1
                shift
                ;;
            --dir)
                if [ $# -lt 2 ]; then die 2 "--dir requires an argument"; fi
                dir="$2"
                shift 2
                ;;
            --)
                shift
                break
                ;;
            -*)
                die 2 "unknown flag: $1"
                ;;
            *)
                if [ -z "$issue" ]; then
                    issue="$1"
                else
                    die 2 "unexpected argument: $1"
                fi
                shift
                ;;
        esac
    done

    validate_issue "$issue"
    local base_dir
    if [ -n "$dir" ]; then
        base_dir="$dir"
    elif [ -n "${AUTOSPEC_SNAPSHOT_DIR:-}" ]; then
        base_dir="$AUTOSPEC_SNAPSHOT_DIR"
    else
        base_dir="/tmp"
    fi
    cmd_get "$issue" "$refresh" "$base_dir"
}

main "$@"
