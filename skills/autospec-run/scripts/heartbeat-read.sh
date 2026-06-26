#!/usr/bin/env bash
# heartbeat-read.sh — Read heartbeat files for the current repo's slug subdir.
#
# Usage:
#   heartbeat-read.sh [--issue <N>] [--repo <owner/repo>]
#
# Without --issue: prints all heartbeat files in the repo's subdir (one path per line).
# With --issue: prints the content of the specific heartbeat file (or empty if not found).
#
# Reads from: ~/.autospec/process-heartbeats/<repo-slug>/ — resolved canonical
# (owner__name) first, with a legacy (owner_name / owner-name) fallback for one
# release so in-flight heartbeats from pre-migration writers are still found.
#
# Environment:
#   AUTOSPEC_HEARTBEAT_DIR   base dir (default: ~/.autospec/process-heartbeats);
#                            AUTOSPEC_WATCHDOG_DIR is honored as a back-compat alias
#   AUTOSPEC_REPO            repo override (owner/repo format)

set -eu

HEARTBEAT_BASE="${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"

# ── Argument parsing ──────────────────────────────────────────────────────────

ISSUE=""
REPO_VAL=""

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)  ISSUE="${2:-}";    shift 2 ;;
        --repo)   REPO_VAL="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'Usage: heartbeat-read.sh [--issue <N>] [--repo <owner/repo>]\n'
            exit 0
            ;;
        *)
            printf 'heartbeat-read: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

# ── Canonical repo-slug helper (F4) ───────────────────────────────────────────
# Source repo-slug.sh so this READER resolves the canonical owner__name dir
# first and falls back to the legacy owner_name / owner-name dirs for one
# release. Resolution order: explicit override → sibling (installed flat
# layout) → AUTOSPEC_SCRIPTS_DIR → repo-relative (dev/test checkout).
_hb_self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
for _rs_cand in \
    "${AUTOSPEC_REPO_SLUG_SH:-}" \
    "${_hb_self_dir}/repo-slug.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
    "${_hb_self_dir}/../../../scripts/repo-slug.sh"; do
    if [ -n "$_rs_cand" ] && [ -f "$_rs_cand" ]; then
        # shellcheck source=/dev/null
        . "$_rs_cand"
        break
    fi
done

# resolve the heartbeat dir for a reader: canonical-first, legacy fallback.
# Degraded inline fallback stays canonical (owner__name) if repo-slug.sh is
# absent so a reader never keys legacy against a canonical writer.
_resolve_slug_dir() {
    if command -v resolve_slug_dir >/dev/null 2>&1; then
        resolve_slug_dir "$1" "$2"
    else
        printf '%s/%s' "$1" "$(printf '%s' "$2" | sed 's#/#__#')"
    fi
}

# ── Resolve repo slug ─────────────────────────────────────────────────────────

resolve_repo() {
    local repo="${1:-}"
    if [ -z "$repo" ] && [ -n "${AUTOSPEC_REPO:-}" ]; then
        repo="$AUTOSPEC_REPO"
    fi
    if [ -z "$repo" ] && command -v gh >/dev/null 2>&1; then
        repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    fi
    if [ -z "$repo" ]; then
        printf 'heartbeat-read: cannot determine repo; set AUTOSPEC_REPO or pass --repo\n' >&2
        exit 1
    fi
    printf '%s' "$repo"
}

REPO_FULL="$(resolve_repo "${REPO_VAL:-}")"
TARGET_DIR="$(_resolve_slug_dir "$HEARTBEAT_BASE" "$REPO_FULL")"

# ── Read heartbeats ───────────────────────────────────────────────────────────

if [ -n "$ISSUE" ]; then
    HB_FILE="${TARGET_DIR}/${ISSUE}.json"
    if [ -f "$HB_FILE" ]; then
        cat "$HB_FILE"
    fi
    exit 0
fi

# List all heartbeat files in the repo's subdir
if [ -d "$TARGET_DIR" ]; then
    for f in "${TARGET_DIR}"/*.json; do
        [ -f "$f" ] || continue
        printf '%s\n' "$f"
    done
fi
