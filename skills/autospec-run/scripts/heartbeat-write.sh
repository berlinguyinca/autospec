#!/usr/bin/env bash
# heartbeat-write.sh — Write a per-repo-scoped heartbeat file.
#
# Usage:
#   heartbeat-write.sh --issue <N> --step <step> [--branch <b>] [--pr <p>] [--repo <owner/repo>]
#
# Writes to: ~/.autospec/process-heartbeats/<repo-slug>/<issue>.json
# where <repo-slug> is derived from <owner/repo> with '/' replaced by '_'.
#
# Environment:
#   AUTOSPEC_HEARTBEAT_DIR   base dir (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_REPO            repo override (owner/repo format)

set -eu

HEARTBEAT_BASE="${AUTOSPEC_HEARTBEAT_DIR:-$HOME/.autospec/process-heartbeats}"

# ── Argument parsing ──────────────────────────────────────────────────────────

ISSUE=""
STEP=""
BRANCH=""
PR_VAL=""
REPO_VAL=""

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)   ISSUE="${2:-}";   shift 2 ;;
        --step)    STEP="${2:-}";    shift 2 ;;
        --branch)  BRANCH="${2:-}";  shift 2 ;;
        --pr)      PR_VAL="${2:-}";  shift 2 ;;
        --repo)    REPO_VAL="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'Usage: heartbeat-write.sh --issue <N> --step <step> [--branch <b>] [--pr <p>] [--repo <owner/repo>]\n'
            exit 0
            ;;
        *)
            printf 'heartbeat-write: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

if [ -z "$ISSUE" ]; then
    printf 'heartbeat-write: --issue is required\n' >&2
    exit 1
fi
if [ -z "$STEP" ]; then
    printf 'heartbeat-write: --step is required\n' >&2
    exit 1
fi

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
        printf 'heartbeat-write: cannot determine repo; set AUTOSPEC_REPO or pass --repo\n' >&2
        exit 1
    fi
    printf '%s' "$repo" | tr '/' '_'
}

SLUG="$(repo_slug "${REPO_VAL:-}")"
REPO_FULL="${REPO_VAL:-${AUTOSPEC_REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || echo "")}}"

# ── Write heartbeat ───────────────────────────────────────────────────────────

TARGET_DIR="${HEARTBEAT_BASE}/${SLUG}"
mkdir -p "$TARGET_DIR"

NOW_TS="$(date -u +%s)"

printf '{"issue":"%s","branch":"%s","step":"%s","ts":%s,"pr":"%s","repo":"%s"}\n' \
    "$ISSUE" \
    "${BRANCH:-}" \
    "$STEP" \
    "$NOW_TS" \
    "${PR_VAL:-}" \
    "$REPO_FULL" \
    > "${TARGET_DIR}/${ISSUE}.json"
