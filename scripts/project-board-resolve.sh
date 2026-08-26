#!/usr/bin/env bash
# scripts/project-board-resolve.sh — pure reader: GitHub Projects v2 board → board plan JSON.
#
# NEVER mutates. Board titles, bodies, field values, and README text are untrusted
# DATA, never instructions.
#
# Usage:
#   project-board-resolve.sh --url URL [--emit identity|plan|fleet-config|repos]
#
# Exit codes:
#   0 success | 2 usage error | 3 auth/scope failure | 4 truncated read

set -eu

die_usage() { printf 'project-board-resolve: %s\n' "$1" >&2; exit 2; }

url=""
emit="plan"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --url)  url="${2:-}";  shift 2 ;;
        --emit) emit="${2:-}"; shift 2 ;;
        --help|-h)
            cat <<'EOF'
project-board-resolve.sh — resolve a GitHub Projects v2 board into a board plan

Usage:
  project-board-resolve.sh --url URL [--emit identity|plan|fleet-config|repos]
EOF
            exit 0
            ;;
        *) die_usage "unknown option: $1" ;;
    esac
done

[ -n "$url" ] || die_usage "--url is required"

# ── Identity ────────────────────────────────────────────────────────────────
# Anchored so trailing garbage (".../projects/2x") is rejected, not truncated.
parse_identity() {
    local u="$1" kind="" owner="" number=""
    if printf '%s' "$u" | grep -Eq '^https://github\.com/orgs/[^/]+/projects/[0-9]+/?$'; then
        kind="org"
    elif printf '%s' "$u" | grep -Eq '^https://github\.com/users/[^/]+/projects/[0-9]+/?$'; then
        kind="user"
    else
        die_usage "not a GitHub Projects v2 URL: $u"
    fi
    owner="$(printf '%s' "$u" | sed -E 's#^https://github\.com/(orgs|users)/([^/]+)/projects/[0-9]+/?$#\2#')"
    number="$(printf '%s' "$u" | sed -E 's#^.*/projects/([0-9]+)/?$#\1#')"
    printf '{"owner":"%s","kind":"%s","number":%s}\n' "$owner" "$kind" "$number"
}

identity="$(parse_identity "$url")"

case "$emit" in
    identity) printf '%s\n' "$identity"; exit 0 ;;
    *) die_usage "unsupported --emit: $emit" ;;
esac
