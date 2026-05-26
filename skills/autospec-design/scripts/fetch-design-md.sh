#!/usr/bin/env bash
# fetch-design-md.sh — fetch + cache per-vendor DESIGN.md from the
# berlinguyinca/awesome-design-md catalog.
#
# Spec: docs/specs/2026-05-26-autospec-design-skill.md § Catalog access.
#
# Usage:
#   fetch-design-md.sh <vendor>
#
# Behavior:
#   - Caches result under "$AUTOSPEC_DESIGN_CACHE_DIR/<vendor>/DESIGN.md"
#     (default $HOME/.autospec/design-cache) with a 24h freshness window
#     ($AUTOSPEC_DESIGN_CACHE_TTL seconds, default 86400).
#   - Fetches via `gh api` first; falls back to `curl -fsSL` against
#     raw.githubusercontent.com. If neither tool is available, exits
#     non-zero with an install hint.
#   - On 404 (missing vendor), lists up to 5 closest vendor names from the
#     catalog directory using simple Levenshtein-style scoring.
#
# Environment:
#   AUTOSPEC_DESIGN_CATALOG_OWNER  (default: berlinguyinca)
#   AUTOSPEC_DESIGN_CATALOG_REPO   (default: awesome-design-md)
#   AUTOSPEC_DESIGN_CATALOG_REF    (default: main)
#   AUTOSPEC_DESIGN_CACHE_DIR      (default: $HOME/.autospec/design-cache)
#   AUTOSPEC_DESIGN_CACHE_TTL      (default: 86400)
#
# Exit codes:
#   0  Success — body printed to stdout.
#   2  Usage error or unknown vendor (with Levenshtein hints).
#   3  Both gh and curl unavailable (install hint surfaced).
#   4  Network/upstream failure with no usable cache.

set -u

OWNER="${AUTOSPEC_DESIGN_CATALOG_OWNER:-berlinguyinca}"
REPO="${AUTOSPEC_DESIGN_CATALOG_REPO:-awesome-design-md}"
REF="${AUTOSPEC_DESIGN_CATALOG_REF:-main}"
CACHE_TTL="${AUTOSPEC_DESIGN_CACHE_TTL:-86400}"
CACHE_DIR="${AUTOSPEC_DESIGN_CACHE_DIR:-$HOME/.autospec/design-cache}"

VENDOR="${1:-}"
if [ -z "$VENDOR" ]; then
    printf 'usage: fetch-design-md.sh <vendor>\n' >&2
    exit 2
fi

cache_file="$CACHE_DIR/$VENDOR/DESIGN.md"

# ── Cache freshness check ────────────────────────────────────────────────────
cache_is_fresh() {
    [ -f "$cache_file" ] || return 1
    local now mtime age
    now="$(date +%s)"
    # GNU stat vs. BSD stat fallback.
    mtime="$(stat -c %Y "$cache_file" 2>/dev/null || stat -f %m "$cache_file" 2>/dev/null || printf 0)"
    age=$((now - mtime))
    [ "$age" -lt "$CACHE_TTL" ]
}

if cache_is_fresh; then
    cat "$cache_file"
    exit 0
fi

# ── Backend probes ───────────────────────────────────────────────────────────
have_gh=0
have_curl=0
if command -v gh > /dev/null 2>&1; then have_gh=1; fi
if command -v curl > /dev/null 2>&1; then have_curl=1; fi

if [ "$have_gh" -eq 0 ] && [ "$have_curl" -eq 0 ]; then
    cat >&2 <<'EOF'
fetch-design-md: neither gh nor curl is available.
Install one of:
  - GitHub CLI: https://cli.github.com/
  - curl:       your distro package manager (apt / brew / pacman / ...)
EOF
    exit 3
fi

# ── Fetch via gh api ─────────────────────────────────────────────────────────
gh_fetch() {
    local vendor="$1"
    gh api \
        "repos/$OWNER/$REPO/contents/design-md/$vendor/DESIGN.md?ref=$REF" \
        --jq '.content' 2> /dev/null \
        | base64 -d 2> /dev/null
}

# ── Fetch via curl ───────────────────────────────────────────────────────────
curl_fetch() {
    local vendor="$1"
    curl -fsSL \
        "https://raw.githubusercontent.com/$OWNER/$REPO/$REF/design-md/$vendor/DESIGN.md" \
        2> /dev/null
}

# ── List catalog vendors (used for Levenshtein hints) ────────────────────────
list_vendors() {
    if [ "$have_gh" -eq 1 ]; then
        gh api "repos/$OWNER/$REPO/contents/design-md?ref=$REF" \
            --jq '.[] | select(.type=="dir") | .name' 2> /dev/null
    elif [ "$have_curl" -eq 1 ]; then
        # GitHub's REST API works unauthenticated for public repos.
        curl -fsSL \
            "https://api.github.com/repos/$OWNER/$REPO/contents/design-md?ref=$REF" \
            2> /dev/null \
            | grep -oE '"name":[[:space:]]*"[^"]+"' \
            | sed -E 's/.*"name":[[:space:]]*"([^"]+)".*/\1/'
    fi
}

# ── Simple Levenshtein distance for closest-vendor hints ─────────────────────
# Bash-only implementation — O(len(a)*len(b)) but vendor names are short.
levenshtein() {
    local a="$1" b="$2"
    local la=${#a} lb=${#b}
    local i j cost
    if [ "$la" -eq 0 ]; then printf '%d' "$lb"; return; fi
    if [ "$lb" -eq 0 ]; then printf '%d' "$la"; return; fi

    # Initialize rows.
    declare -a prev curr
    for j in $(seq 0 "$lb"); do prev[$j]=$j; done

    for ((i = 1; i <= la; i++)); do
        curr[0]=$i
        for ((j = 1; j <= lb; j++)); do
            if [ "${a:i-1:1}" = "${b:j-1:1}" ]; then
                cost=0
            else
                cost=1
            fi
            local del=$((prev[j] + 1))
            local ins=$((curr[j-1] + 1))
            local sub=$((prev[j-1] + cost))
            local m=$del
            [ "$ins" -lt "$m" ] && m=$ins
            [ "$sub" -lt "$m" ] && m=$sub
            curr[$j]=$m
        done
        for j in $(seq 0 "$lb"); do prev[$j]="${curr[$j]}"; done
    done
    printf '%d' "${prev[$lb]}"
}

print_levenshtein_hints() {
    local target="$1"
    local vendors
    vendors="$(list_vendors)"
    if [ -z "$vendors" ]; then
        printf 'fetch-design-md: vendor "%s" not found and catalog listing unavailable.\n' \
            "$target" >&2
        return
    fi
    printf 'fetch-design-md: vendor "%s" not found. Closest matches:\n' "$target" >&2
    {
        while IFS= read -r v; do
            [ -z "$v" ] && continue
            d="$(levenshtein "$target" "$v")"
            printf '%s\t%s\n' "$d" "$v"
        done <<< "$vendors"
    } | sort -n | head -5 | awk '{ printf "  - %s\n", $2 }' >&2
}

# ── Attempt fetch (gh first, curl fallback) ──────────────────────────────────
body=""
fetch_ok=0

if [ "$have_gh" -eq 1 ]; then
    body="$(gh_fetch "$VENDOR" || true)"
    if [ -n "$body" ]; then fetch_ok=1; fi
fi

if [ "$fetch_ok" -eq 0 ] && [ "$have_curl" -eq 1 ]; then
    body="$(curl_fetch "$VENDOR" || true)"
    if [ -n "$body" ]; then fetch_ok=1; fi
fi

if [ "$fetch_ok" -eq 0 ]; then
    # Distinguish missing-vendor (404) from generic network failure by
    # attempting a catalog listing. If we got a listing, the vendor name
    # is wrong; otherwise it's a network problem.
    if [ -n "$(list_vendors)" ]; then
        print_levenshtein_hints "$VENDOR"
        exit 2
    fi
    printf 'fetch-design-md: failed to fetch DESIGN.md for "%s" from %s/%s@%s.\n' \
        "$VENDOR" "$OWNER" "$REPO" "$REF" >&2
    printf 'fetch-design-md: catalog listing unreachable — check network and credentials.\n' >&2
    exit 4
fi

# ── Cache + emit ─────────────────────────────────────────────────────────────
mkdir -p "$(dirname "$cache_file")"
printf '%s' "$body" > "$cache_file"
printf '%s' "$body"
