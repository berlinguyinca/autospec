#!/usr/bin/env bash
# score-suggestion.sh — deterministic rubric scorer for /autospec-design suggest.
#
# Spec: docs/specs/2026-05-26-autospec-design-skill.md § API shape → suggest.
#
# Scans a repo for framework / brand / domain signals, scores every catalog
# vendor against the repo using a fixed rubric, and prints the top 3 with a
# one-line rationale. Read-only — never modifies any file in the target repo.
#
# Usage:
#   score-suggestion.sh <repo-root>
#
# Rubric (cap 6 per vendor):
#   +2  framework match  — repo uses a web/JS UI framework AND the vendor is a
#                          developer / SaaS / web-product brand.
#   +1  brand match      — vendor's normalized name appears in the repo's
#                          README or package.json.
#   +1  domain match     — keyword overlap between the vendor name tokens and
#                          the repo README.
#
# Output: tab-separated "<score>\t<vendor>\t<rationale>" lines, top 3 by score
# (descending), highest first. Vendors scoring 0 are still eligible if fewer
# than 3 vendors score above 0.
#
# Environment:
#   AUTOSPEC_DESIGN_VENDORS  — space/newline-separated vendor list. When set,
#                              used instead of fetching the catalog directory
#                              (keeps the helper testable offline and lets the
#                              suggest flow pass a pre-fetched list).
#   AUTOSPEC_DESIGN_CATALOG_OWNER / _REPO / _REF — catalog coordinates when
#                              fetching the vendor list (see fetch-design-md.sh).
#
# Exit codes:
#   0  Success.
#   1  Usage error / repo root missing.
#   4  Catalog vendor list unavailable (no AUTOSPEC_DESIGN_VENDORS and no
#      gh/curl reachable).

set -u

OWNER="${AUTOSPEC_DESIGN_CATALOG_OWNER:-berlinguyinca}"
REPO="${AUTOSPEC_DESIGN_CATALOG_REPO:-awesome-design-md}"
REF="${AUTOSPEC_DESIGN_CATALOG_REF:-main}"

REPO_ROOT="${1:-}"
if [ -z "$REPO_ROOT" ]; then
    printf 'usage: score-suggestion.sh <repo-root>\n' >&2
    exit 1
fi
if [ ! -d "$REPO_ROOT" ]; then
    printf 'score-suggestion: repo root not found: %s\n' "$REPO_ROOT" >&2
    exit 1
fi

# Developer / SaaS / web-product brands in the catalog. A repo built on a
# web/JS UI framework aligns with these design languages (per spec: "Linear" +
# "developer tool" maps well to a Next.js repo).
WEB_PRODUCT_VENDORS=" linear vercel stripe notion figma cursor supabase \
    sentry posthog framer raycast warp resend sanity mintlify intercom \
    slack shopify airtable cal claude cohere replicate together mistral \
    minimax elevenlabs opencode lovable miro zapier wise revolut x.ai \
    composio clay clickhouse mongodb webflow superhuman expo voltagent "

# ── Repo signal detection ────────────────────────────────────────────────────

# detect_framework — print "web" when a JS/TS UI framework is present, else "".
detect_framework() {
    local root="$1"
    # Config-file signatures.
    if ls "$root"/next.config.* "$root"/vite.config.* "$root"/angular.json \
        "$root"/svelte.config.* "$root"/tailwind.config.* > /dev/null 2>&1; then
        printf 'web'
        return
    fi
    # package.json dependency signatures.
    if [ -f "$root/package.json" ]; then
        if grep -qE '"(next|react|vue|svelte|@angular/core|vite|tailwindcss)"' \
            "$root/package.json" 2> /dev/null; then
            printf 'web'
            return
        fi
    fi
    printf ''
}

# repo_text — concatenated lower-cased README + package.json text for keyword
# matching. Printed once and reused.
repo_text() {
    local root="$1"
    {
        [ -f "$root/README.md" ] && cat "$root/README.md"
        [ -f "$root/readme.md" ] && cat "$root/readme.md"
        [ -f "$root/package.json" ] && cat "$root/package.json"
    } 2> /dev/null | tr '[:upper:]' '[:lower:]'
}

# normalize_vendor — strip common TLD-style suffixes so "linear.app" → "linear".
normalize_vendor() {
    local v="$1"
    v="${v%.app}"
    v="${v%.ai}"
    v="${v%.com}"
    v="${v%.io}"
    printf '%s' "$v"
}

# ── Scoring ──────────────────────────────────────────────────────────────────

FRAMEWORK="$(detect_framework "$REPO_ROOT")"
TEXT="$(repo_text "$REPO_ROOT" | tr '[:lower:]' '[:lower:]')"

# score_vendor VENDOR — print "<score>\t<vendor>\t<rationale>".
score_vendor() {
    local vendor="$1"
    local norm
    norm="$(normalize_vendor "$vendor" | tr '[:upper:]' '[:lower:]')"
    local score=0
    local parts=""

    # Framework match (+2): web framework present AND vendor is a web product.
    if [ -n "$FRAMEWORK" ]; then
        case "$WEB_PRODUCT_VENDORS" in
            *" $norm "*)
                score=$((score + 2))
                parts="${parts}framework(+2) "
                ;;
        esac
    fi

    # Brand match (+1): normalized vendor name appears in repo text.
    if [ -n "$norm" ] && printf '%s' "$TEXT" | grep -qF "$norm"; then
        score=$((score + 1))
        parts="${parts}brand(+1) "
    fi

    # Domain match (+1): any vendor-name token (len >= 4) overlaps repo text,
    # distinct from a full brand hit (covers multi-word / partial overlap).
    local token matched_domain=0
    for token in $(printf '%s' "$norm" | tr -c 'a-z0-9' ' '); do
        [ "${#token}" -ge 4 ] || continue
        if printf '%s' "$TEXT" | grep -qF "$token"; then
            matched_domain=1
            break
        fi
    done
    if [ "$matched_domain" -eq 1 ]; then
        # Only add domain credit when it is not already counted as a brand hit.
        case "$parts" in
            *"brand(+1)"*) : ;;
            *)
                score=$((score + 1))
                parts="${parts}domain(+1) "
                ;;
        esac
    fi

    # Cap at 6.
    [ "$score" -gt 6 ] && score=6

    [ -z "$parts" ] && parts="default(+0)"
    printf '%s\t%s\t%s\n' "$score" "$vendor" "${parts% }"
}

# ── Vendor list source ───────────────────────────────────────────────────────

vendor_list() {
    if [ -n "${AUTOSPEC_DESIGN_VENDORS:-}" ]; then
        printf '%s\n' $AUTOSPEC_DESIGN_VENDORS
        return 0
    fi
    if command -v gh > /dev/null 2>&1; then
        gh api "repos/$OWNER/$REPO/contents/design-md?ref=$REF" \
            --jq '.[] | select(.type=="dir") | .name' 2> /dev/null && return 0
    fi
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL \
            "https://api.github.com/repos/$OWNER/$REPO/contents/design-md?ref=$REF" \
            2> /dev/null \
            | grep -oE '"name":[[:space:]]*"[^"]+"' \
            | sed -E 's/.*"name":[[:space:]]*"([^"]+)".*/\1/' && return 0
    fi
    return 4
}

VENDORS="$(vendor_list)"
if [ -z "$VENDORS" ]; then
    printf 'score-suggestion: catalog vendor list unavailable.\n' >&2
    printf 'score-suggestion: set AUTOSPEC_DESIGN_VENDORS or install gh/curl.\n' >&2
    exit 4
fi

# Score every vendor, sort by score DESC (stable on ties via vendor name), top 3.
{
    while IFS= read -r v; do
        [ -z "$v" ] && continue
        score_vendor "$v"
    done <<EOF
$VENDORS
EOF
} | sort -t "$(printf '\t')" -k1,1nr -k2,2 | head -3
