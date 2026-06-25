#!/usr/bin/env bash
# scripts/repo-slug.sh — canonical repo-slug helper
#
# Maps owner/name → owner__name (double-underscore canonical form).
# The double underscore disambiguates repos whose names contain a single `_`.
#
# Functions (source-able):
#   canonical_slug  owner/name  → "owner__name" (exits non-zero on bad input)
#   resolve_slug_dir base owner/name → best matching dir under base:
#       1. base/owner__name  (canonical)
#       2. base/owner_name   (legacy — one-release compat)
#       3. base/owner-name   (legacy — one-release compat)
#       Falls back to canonical path when none of the dirs exist.
#
# Standalone usage:
#   bash scripts/repo-slug.sh [--canonical] owner/name
#   bash scripts/repo-slug.sh --resolve-dir <base> owner/name
#
# No reuse of the existing `tr '/' '_'` one-liner in autospec-run-registry.sh
# or autospec-watchdog.sh — those produce legacy forms (single _) and are the
# callers that MUST migrate to this helper; this file IS the canonical source.

set -euo pipefail

# ---------------------------------------------------------------------------
# canonical_slug owner/name → owner__name
# Exits 1 with a message on stderr if input lacks exactly one '/'.
# ---------------------------------------------------------------------------
canonical_slug() {
    local input="${1:-}"
    if [ -z "$input" ]; then
        printf 'repo-slug: input must be non-empty (expected owner/name)\n' >&2
        return 1
    fi
    # Count slashes — must be exactly one
    local slash_count
    slash_count="$(printf '%s' "$input" | tr -cd '/' | wc -c | tr -d ' ')"
    if [ "$slash_count" -ne 1 ]; then
        printf 'repo-slug: input "%s" must contain exactly one "/" (got %s)\n' \
            "$input" "$slash_count" >&2
        return 1
    fi
    # Replace the single / with __ (double underscore)
    printf '%s' "$input" | sed 's#/#__#'
}

# ---------------------------------------------------------------------------
# resolve_slug_dir base owner/name → path
# Returns the best existing dir under base that corresponds to the repo.
# Priority: canonical (owner__name) > legacy-underscore (owner_name)
#           > legacy-hyphen (owner-name). Falls back to canonical when
#           none of the directories exist (new-repo / first write path).
# ---------------------------------------------------------------------------
resolve_slug_dir() {
    local base="${1:-}"
    local repo="${2:-}"
    if [ -z "$base" ] || [ -z "$repo" ]; then
        printf 'repo-slug: resolve_slug_dir requires base and owner/name\n' >&2
        return 1
    fi

    local owner name
    owner="${repo%%/*}"
    name="${repo##*/}"

    local canonical_dir="${base}/${owner}__${name}"
    local legacy_under_dir="${base}/${owner}_${name}"
    local legacy_hyphen_dir="${base}/${owner}-${name}"

    if [ -d "$canonical_dir" ]; then
        printf '%s' "$canonical_dir"
    elif [ -d "$legacy_under_dir" ]; then
        printf '%s' "$legacy_under_dir"
    elif [ -d "$legacy_hyphen_dir" ]; then
        printf '%s' "$legacy_hyphen_dir"
    else
        # No existing dir — return canonical (caller creates it)
        printf '%s' "$canonical_dir"
    fi
}

# ---------------------------------------------------------------------------
# Standalone entry point
# ---------------------------------------------------------------------------
_repo_slug_main() {
    local mode="canonical"
    local base=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --canonical)
                mode="canonical"
                shift
                ;;
            --resolve-dir)
                mode="resolve-dir"
                shift
                base="${1:-}"
                shift
                ;;
            --)
                shift
                break
                ;;
            -*)
                printf 'repo-slug: unknown flag: %s\n' "$1" >&2
                return 1
                ;;
            *)
                break
                ;;
        esac
    done

    local repo="${1:-}"

    case "$mode" in
        canonical)
            canonical_slug "$repo"
            ;;
        resolve-dir)
            resolve_slug_dir "$base" "$repo"
            ;;
    esac
}

# Source-guard: only run main when executed directly, not when sourced.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    _repo_slug_main "$@"
fi
