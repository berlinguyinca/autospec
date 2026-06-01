#!/usr/bin/env bash
# release-area-dispatch.sh — autospec-release area dispatcher (issue #731).
#
# Reads the 6 area definitions from skills/autospec-release/areas/, dispatches
# one subagent per area (harness-aware via scripts/lib/autospec-harness-detect.sh,
# PR #725), then aggregates each area's findings into a single
# .autospec/release-verdict.json honoring the schema consumed by
# scripts/compute-release-verdict.sh (PR #636).
#
# Verify-first filter (PR #650): per-area findings pass through
# scripts/qa-finding-filter.sh before aggregation so unverified noise does
# not reach the verdict.
#
# Modes:
#   --list                  list the 6 area names and exit 0.
#   --area <name>           print the area's definition file path and exit 0.
#   --aggregate <dir>       merge per-area finding JSON files in <dir> into
#                           .autospec/release-verdict.json and exit 0.
#   (no args)               full dispatch + aggregate.
#
# Env:
#   AUTOSPEC_RELEASE_AREAS_DIR — override skills/autospec-release/areas/ root.
#   AUTOSPEC_RELEASE_VERDICT   — override .autospec/release-verdict.json path.
#   AUTOSPEC_RELEASE_DISPATCH_CMD — override dispatch command (test hook).
#                                   Invoked as `$cmd <area-name> <area-file>`.
#                                   Must emit a single JSON object on stdout
#                                   with at least: {area, status,
#                                   release_blocking, summary, findings[]}.
#   AUTOSPEC_RELEASE_HEAD_SHA  — override `git rev-parse HEAD` (test hook).
#
# Exit codes:
#   0 success.
#   2 missing area file.
#   3 jq or git unavailable.
#   4 dispatcher failed.

set -u

REPO_ROOT="${AUTOSPEC_RELEASE_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
AREAS_DIR="${AUTOSPEC_RELEASE_AREAS_DIR:-$REPO_ROOT/skills/autospec-release/areas}"
VERDICT_FILE="${AUTOSPEC_RELEASE_VERDICT:-$REPO_ROOT/.autospec/release-verdict.json}"

AREAS=(
    spec-completeness
    docs-freshness
    implementation-completeness
    test-coverage
    qa-artifact-integrity
    legacy-cleanup
)

usage() {
    sed -n '2,30p' "$0"
}

require() {
    command -v "$1" >/dev/null 2>&1 || {
        printf 'release-area-dispatch: required binary missing: %s\n' "$1" >&2
        exit 3
    }
}

list_areas() {
    printf '%s\n' "${AREAS[@]}"
}

area_file() {
    local name="$1"
    local path="$AREAS_DIR/$name.md"
    if [ ! -f "$path" ]; then
        printf 'release-area-dispatch: area definition missing: %s\n' "$path" >&2
        exit 2
    fi
    printf '%s\n' "$path"
}

resolve_head_sha() {
    if [ -n "${AUTOSPEC_RELEASE_HEAD_SHA:-}" ]; then
        printf '%s' "$AUTOSPEC_RELEASE_HEAD_SHA"
        return 0
    fi
    git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null
}

# apply_verify_first_filter
# Honor verify-first discipline (PR #650) on the aggregated release-verdict.
# Each area subagent is its own verifier, so synthetic findings carry
# verified_at == head_sha and pass the filter as-verified. When the operator
# wants the qa-finding-filter (PR #650) to re-probe stale findings, they set
# AUTOSPEC_RELEASE_VERIFY_FIRST=1 — by default the filter is skipped because
# its qa-verify-finding.sh probes target QA categories, not release areas.
apply_verify_first_filter() {
    [ "${AUTOSPEC_RELEASE_VERIFY_FIRST:-0}" = "1" ] || return 0
    local filter="$REPO_ROOT/scripts/qa-finding-filter.sh"
    [ -x "$filter" ] || return 0
    AUTOSPEC_QA_STRICT="${AUTOSPEC_QA_STRICT:-0}" \
        "$filter" --verdict "$VERDICT_FILE" >/dev/null 2>&1 || true
}

dispatch_one_area() {
    local name="$1"
    local outdir="$2"
    local file
    file="$(area_file "$name")"
    local out="$outdir/$name.json"
    if [ -n "${AUTOSPEC_RELEASE_DISPATCH_CMD:-}" ]; then
        # Test/integration hook — caller-provided dispatcher.
        if ! eval "$AUTOSPEC_RELEASE_DISPATCH_CMD \"$name\" \"$file\"" \
            > "$out"; then
            return 4
        fi
    else
        # Production path — delegate to harness-aware loop dispatcher.
        # shellcheck disable=SC1091
        . "$REPO_ROOT/scripts/lib/autospec-harness-detect.sh"
        autospec_harness_resolve_dispatcher || return 4
        local prompt
        prompt="Read area definition $file. Audit the repo per its scope, then \
emit exactly one JSON object on stdout with: area, status (PASS|PARTIAL|FAIL|NOT_TESTED), \
release_blocking (bool), summary, findings[]. No prose."
        if ! autospec_harness_invoke autonomous "$prompt" \
            > "$out"; then
            return 4
        fi
    fi
    [ -s "$out" ] || {
        printf 'release-area-dispatch: dispatcher returned empty output for %s\n' "$name" >&2
        return 4
    }
    return 0
}

aggregate() {
    local indir="$1"
    require jq
    local head_sha
    head_sha="$(resolve_head_sha)"
    if [ -z "$head_sha" ]; then
        printf 'release-area-dispatch: cannot resolve HEAD sha\n' >&2
        exit 3
    fi
    mkdir -p "$(dirname "$VERDICT_FILE")"
    # Merge: each per-area JSON contributes a row in findings[], carrying its
    # area name so consumers can pivot. The aggregate also surfaces
    # live_app_proof (true iff qa-artifact-integrity says so) so the existing
    # compute-release-verdict.sh consumes the merged verdict unchanged.
    local merged
    merged="$(jq -s --arg head "$head_sha" '
        . as $rows
        | {
            head_sha: $head,
            live_app_proof: ([$rows[] | select(.area == "qa-artifact-integrity") | .live_app_proof // false] | .[0] // false),
            findings: ([$rows[] | (.findings // [{
                area: .area,
                status: (.status // "NOT_TESTED"),
                release_blocking: (.release_blocking // false),
                summary: (.summary // "")
            }])[] | .verified_at = (.verified_at // $head)]),
            areas: [$rows[] | {
                area: .area,
                status: (.status // "NOT_TESTED"),
                release_blocking: (.release_blocking // false),
                summary: (.summary // "")
            }]
        }
    ' "$indir"/*.json)"
    if [ -z "$merged" ]; then
        printf 'release-area-dispatch: aggregation produced empty verdict\n' >&2
        exit 4
    fi
    printf '%s\n' "$merged" > "$VERDICT_FILE"
    apply_verify_first_filter
    printf '%s\n' "$VERDICT_FILE"
}

main() {
    case "${1:-}" in
        -h|--help) usage; exit 0 ;;
        --list) list_areas; exit 0 ;;
        --area)
            [ -n "${2:-}" ] || { usage; exit 2; }
            area_file "$2"; exit 0
            ;;
        --aggregate)
            [ -n "${2:-}" ] || { usage; exit 2; }
            aggregate "$2"; exit 0
            ;;
        "" )
            # Full dispatch.
            require jq
            tmp="$(mktemp -d)"
            export _RELEASE_DISPATCH_TMP="$tmp"
            trap 'rm -rf "$_RELEASE_DISPATCH_TMP"' EXIT
            local rc=0
            for a in "${AREAS[@]}"; do
                dispatch_one_area "$a" "$tmp" || rc=$?
            done
            [ "$rc" -eq 0 ] || exit "$rc"
            aggregate "$tmp"
            ;;
        *) usage; exit 2 ;;
    esac
}

main "$@"
