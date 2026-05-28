#!/usr/bin/env bash
# Shared helpers for autospec-fleet shell commands.

if [ -z "${BASH_VERSION:-}" ]; then
    printf 'fleet-lib.sh must be sourced by bash\n' >&2
    return 2 2>/dev/null || exit 2
fi

normalize_repo_url() {
    local input="${1:-}"
    local path=""
    local owner=""
    local repo=""

    case "$input" in
        https://github.com/*/*)
            path="${input#https://github.com/}"
            ;;
        git@github.com:*/*)
            path="${input#git@github.com:}"
            ;;
        *)
            printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
            return 2
            ;;
    esac

    path="${path%.git}"
    if [[ "$path" =~ ^([^/]+)/([^/]+)$ ]]; then
        owner="${BASH_REMATCH[1]}"
        repo="${BASH_REMATCH[2]}"
    else
        printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
        return 2
    fi

    if [ -z "$owner" ] || [ -z "$repo" ]; then
        printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
        return 2
    fi

    printf '%s/%s\n' "$owner" "$repo"
}

repo_slug() {
    local normalized="${1:-}"

    case "$normalized" in
        */*) ;;
        *)
            printf 'fleet: repo slug requires owner/repo, got: %s\n' "$normalized" >&2
            return 2
            ;;
    esac

    printf '%s\n' "${normalized//\//__}"
}

repo_checkout_path() {
    local workspace="${1:-}"
    local normalized="${2:-}"
    local slug

    if [ -z "$workspace" ]; then
        printf 'fleet: workspace is required\n' >&2
        return 2
    fi

    slug="$(repo_slug "$normalized")" || return 2
    printf '%s/%s\n' "${workspace%/}" "$slug"
}
