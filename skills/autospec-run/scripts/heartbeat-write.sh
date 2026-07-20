#!/usr/bin/env bash
# heartbeat-write.sh — Write a per-repo-scoped heartbeat file.
#
# Usage:
#   heartbeat-write.sh --issue <N> --step <step> [--branch <b>] [--pr <p>] [--repo <owner/repo>]
#                      [--worker-id <id> --claim-id <id> --session-id <id>]
#
# Writes to: ~/.autospec/process-heartbeats/<repo-slug>/<issue>.json
# Recovery-bound writes also atomically update
# ~/.autospec/process-heartbeats/<repo-slug>/sessions/<hex-session-id>.json.
# where <repo-slug> is the canonical owner__name form produced by
# scripts/repo-slug.sh (F4). Readers resolve canonical-first with a legacy
# (owner_name) fallback for one release so in-flight heartbeats aren't orphaned.
#
# Environment:
#   AUTOSPEC_HEARTBEAT_DIR   base dir (default: ~/.autospec/process-heartbeats);
#                            AUTOSPEC_WATCHDOG_DIR is honored as a back-compat alias
#   AUTOSPEC_REPO            repo override (owner/repo format)

set -eu

# Honor both env-var names so writers and the watchdog never point at different
# dirs (a mismatch silently hid every heartbeat from the watchdog).
HEARTBEAT_BASE="${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"

# ── Argument parsing ──────────────────────────────────────────────────────────

ISSUE=""
STEP=""
BRANCH=""
PR_VAL=""
REPO_VAL=""
WORKER_ID=""
CLAIM_ID=""
SESSION_ID=""

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)   ISSUE="${2:-}";   shift 2 ;;
        --step)    STEP="${2:-}";    shift 2 ;;
        --branch)  BRANCH="${2:-}";  shift 2 ;;
        --pr)      PR_VAL="${2:-}";  shift 2 ;;
        --repo)    REPO_VAL="${2:-}"; shift 2 ;;
        --worker-id) WORKER_ID="${2:-}"; shift 2 ;;
        --claim-id) CLAIM_ID="${2:-}"; shift 2 ;;
        --session-id) SESSION_ID="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'Usage: heartbeat-write.sh --issue <N> --step <step> [--branch <b>] [--pr <p>] [--repo <owner/repo>] [--worker-id <id> --claim-id <id> --session-id <id>]\n'
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

binding_fields=0
[ -n "$WORKER_ID" ] && binding_fields=$((binding_fields + 1))
[ -n "$CLAIM_ID" ] && binding_fields=$((binding_fields + 1))
[ -n "$SESSION_ID" ] && binding_fields=$((binding_fields + 1))
if [ "$binding_fields" -ne 0 ] && [ "$binding_fields" -ne 3 ]; then
    printf 'heartbeat-write: session binding requires --session-id, --claim-id, and --worker-id\n' >&2
    exit 1
fi

# ── Canonical repo-slug helper (F4) ───────────────────────────────────────────
# Source repo-slug.sh so this WRITER emits the canonical owner__name slug
# (matching the watchdog/heartbeat-read resolvers). Resolution order: explicit
# override → sibling (installed flat layout) → AUTOSPEC_SCRIPTS_DIR →
# repo-relative (dev/test checkout).
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

# canonical slug for a writer; degraded inline fallback stays canonical
# (owner__name) so we never silently regress to the legacy single-underscore
# form if repo-slug.sh is somehow absent.
_canonical_slug() {
    if command -v canonical_slug >/dev/null 2>&1; then
        canonical_slug "$1"
    else
        printf '%s' "$1" | sed 's#/#__#'
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
        printf 'heartbeat-write: cannot determine repo; set AUTOSPEC_REPO or pass --repo\n' >&2
        exit 1
    fi
    printf '%s' "$repo"
}

REPO_FULL="$(resolve_repo "${REPO_VAL:-}")"
SLUG="$(_canonical_slug "$REPO_FULL")"

# ── Write heartbeat ───────────────────────────────────────────────────────────

TARGET_DIR="${HEARTBEAT_BASE}/${SLUG}"
mkdir -p "$TARGET_DIR"

NOW_TS="$(date -u +%s)"

# host: the machine that owns this heartbeat. Added backward-compatibly for the
# cross-host partial-resume gate (resume-scan.sh). Readers that predate this
# field, or any heartbeat written before it existed, simply see a missing
# "host" key and MUST treat it as an unknown host → never eligible for
# --resume-partial (always clean-restart). Override via AUTOSPEC_HOST for tests.
HOST_VAL="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"

if [ "$binding_fields" -eq 3 ]; then
    printf '{"issue":"%s","branch":"%s","step":"%s","ts":%s,"pr":"%s","repo":"%s","host":"%s","worker_id":"%s","claim_id":"%s","session_id":"%s"}\n' \
        "$ISSUE" "${BRANCH:-}" "$STEP" "$NOW_TS" "${PR_VAL:-}" "$REPO_FULL" "$HOST_VAL" \
        "$WORKER_ID" "$CLAIM_ID" "$SESSION_ID" > "${TARGET_DIR}/${ISSUE}.json"

    session_key="$(LC_ALL=C printf '%s' "$SESSION_ID" | od -An -tx1 | tr -d ' \n')"
    [ -n "$session_key" ] || { printf 'heartbeat-write: --session-id must not be empty\n' >&2; exit 1; }
    session_dir="${TARGET_DIR}/sessions"
    session_file="${session_dir}/${session_key}.json"
    session_tmp="${session_file}.tmp.$$"
    mkdir -p "$session_dir"
    cp "${TARGET_DIR}/${ISSUE}.json" "$session_tmp"
    mv "$session_tmp" "$session_file"
else
    printf '{"issue":"%s","branch":"%s","step":"%s","ts":%s,"pr":"%s","repo":"%s","host":"%s"}\n' \
        "$ISSUE" "${BRANCH:-}" "$STEP" "$NOW_TS" "${PR_VAL:-}" "$REPO_FULL" "$HOST_VAL" \
        > "${TARGET_DIR}/${ISSUE}.json"
fi
