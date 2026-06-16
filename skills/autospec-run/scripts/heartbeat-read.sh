#!/usr/bin/env bash
# heartbeat-read.sh — Read heartbeat files for the current repo's slug subdir.
#
# Usage:
#   heartbeat-read.sh [--issue <N>] [--repo <owner/repo>]
#
# Without --issue: prints all heartbeat files in the repo's subdir (one path per line).
# With --issue: prints the content of the specific heartbeat file (or empty if not found).
#
# Reads from: ~/.autospec/process-heartbeats/<repo-slug>/
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

# ── Resolve repo slug ─────────────────────────────────────────────────────────

repo_slug() {
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
    printf '%s' "$repo" | tr '/' '_'
}

SLUG="$(repo_slug "${REPO_VAL:-}")"
TARGET_DIR="${HEARTBEAT_BASE}/${SLUG}"

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
