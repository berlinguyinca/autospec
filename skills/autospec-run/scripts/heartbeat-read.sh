#!/usr/bin/env bash
# heartbeat-read.sh — Read heartbeat files for the current repo's slug subdir.
#
# Usage:
#   heartbeat-read.sh [--issue <N> | --session-id <id>] [--repo <owner/repo>]
#
# Without --issue: prints all heartbeat files in the repo's subdir (one path per line).
# With --issue: prints the content of the specific heartbeat file (or empty if not found).
# With --session-id: prints only the exact immutable Wait-target binding and
# exits non-zero when the binding is missing, malformed, or legacy/unbound.
#
# Reads from: ~/.autospec/process-heartbeats/<repo-slug>/ — resolved canonical
# Rust collision-safe form first, then owner__name, with a legacy
# (owner_name / owner-name) fallback for one release so in-flight heartbeats
# from pre-migration writers are still found.
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
SESSION_ID=""

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)  ISSUE="${2:-}";    shift 2 ;;
        --repo)   REPO_VAL="${2:-}"; shift 2 ;;
        --session-id) SESSION_ID="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'Usage: heartbeat-read.sh [--issue <N> | --session-id <id>] [--repo <owner/repo>]\n'
            exit 0
            ;;
        *)
            printf 'heartbeat-read: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

if [ -n "$ISSUE" ] && [ -n "$SESSION_ID" ]; then
    printf 'heartbeat-read: --issue and --session-id are mutually exclusive\n' >&2
    exit 1
fi
if [ -n "$ISSUE" ]; then
    case "$ISSUE" in
        *[!0-9]*|0|0*)
            printf 'heartbeat-read: --issue must be a canonical positive integer\n' >&2
            exit 1
            ;;
    esac
fi

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
_slug_dirs() {
    _base="$1"
    _repo="$2"
    _owner="${_repo%%/*}"
    _name="${_repo##*/}"
    _canonical="${_base}/${_owner}__${_name}"
    _collision_safe="${_base}/o${#_owner}_${_owner}_r${#_name}_${_name}"
    _legacy_under="${_base}/${_owner}_${_name}"
    _legacy_hyphen="${_base}/${_owner}-${_name}"
    printf '%s\n' "$_collision_safe"
    [ "$_canonical" = "$_collision_safe" ] || printf '%s\n' "$_canonical"
    [ "$_legacy_under" = "$_canonical" ] || printf '%s\n' "$_legacy_under"
    [ "$_legacy_hyphen" = "$_canonical" ] || [ "$_legacy_hyphen" = "$_legacy_under" ] || printf '%s\n' "$_legacy_hyphen"
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

# ── Read heartbeats ───────────────────────────────────────────────────────────

if [ -n "$SESSION_ID" ]; then
    session_key="$(LC_ALL=C printf '%s' "$SESSION_ID" | od -An -tx1 | tr -d ' \n')"
    for TARGET_DIR in $(_slug_dirs "$HEARTBEAT_BASE" "$REPO_FULL"); do
        binding="${TARGET_DIR}/sessions/${session_key}.json"
        [ -f "$binding" ] || continue
        if ! command -v jq >/dev/null 2>&1; then
            printf 'heartbeat-read: jq is required for exact session binding validation\n' >&2
            exit 2
        fi
        if ! jq -e --arg session_id "$SESSION_ID" \
            '.session_id == $session_id and (.claim_id | type == "string" and length > 0) and (.worker_id | type == "string" and length > 0) and (.issue | type == "string" and length > 0) and (.branch | type == "string")' \
            "$binding" >/dev/null 2>&1; then
            printf 'heartbeat-read: malformed durable heartbeat binding for session %s\n' "$SESSION_ID" >&2
            exit 4
        fi
        cat "$binding"
        exit 0
    done
    printf 'heartbeat-read: no durable heartbeat binding for session %s\n' "$SESSION_ID" >&2
    exit 4
fi

if [ -n "$ISSUE" ]; then
    newest_file=""
    newest_mtime="-1"
    for TARGET_DIR in $(_slug_dirs "$HEARTBEAT_BASE" "$REPO_FULL"); do
        HB_FILE="${TARGET_DIR}/${ISSUE}.json"
        [ -f "$HB_FILE" ] || continue
        mtime="$(stat -c %Y "$HB_FILE" 2>/dev/null || stat -f %m "$HB_FILE" 2>/dev/null || echo 0)"
        case "$mtime" in *[!0-9]*|'') mtime=0 ;; esac
        if [ "$mtime" -gt "$newest_mtime" ]; then
            newest_mtime="$mtime"
            newest_file="$HB_FILE"
        fi
    done
    [ -n "$newest_file" ] && cat "$newest_file"
    exit 0
fi

# List all heartbeat files in compatible repo slug dirs.
seen_files=""
for TARGET_DIR in $(_slug_dirs "$HEARTBEAT_BASE" "$REPO_FULL"); do
    if [ -d "$TARGET_DIR" ]; then
        for f in "${TARGET_DIR}"/*.json; do
            [ -f "$f" ] || continue
            case "$seen_files" in *"
$f
"*) continue ;; esac
            seen_files="${seen_files}
$f
"
            printf '%s\n' "$f"
        done
    fi
done
