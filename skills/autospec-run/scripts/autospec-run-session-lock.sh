#!/usr/bin/env bash
# autospec-run-session-lock.sh — per-session single-instance guard for /autospec-run.
#
# Ensures only ONE autospec-run monitor runs per harness session. The lock is
# keyed by a stable per-session token, so two separate harness sessions still run
# independently (by design); a second invocation WITHIN the same session is
# refused with a status report.
#
#   acquire [--repo <owner/repo>] [--force]   exit 0 acquired | 3 busy (same session)
#   release                                   exit 0
#   status                                    exit 0 (prints lock JSON or active:false)
#
# Session token resolution (first non-empty wins): AUTOSPEC_SESSION_ID,
# CLAUDE_CODE_SESSION_ID, CODEX_SESSION_ID, CODEX_THREAD_ID, OPENCODE_SESSION_ID,
# OPENCODE_SESSION, TERM_SESSION_ID, POSIX session id, then PPID.
set -u

LOCK_DIR="${AUTOSPEC_LOCK_DIR:-$HOME/.autospec/locks}"

session_token() {
    local v
    for v in "${AUTOSPEC_SESSION_ID:-}" "${CLAUDE_CODE_SESSION_ID:-}" \
             "${CODEX_SESSION_ID:-}" "${CODEX_THREAD_ID:-}" \
             "${OPENCODE_SESSION_ID:-}" "${OPENCODE_SESSION:-}" \
             "${TERM_SESSION_ID:-}"; do
        if [ -n "$v" ]; then printf '%s' "$v"; return 0; fi
    done
    local sid
    sid="$(ps -o sess= -p "$$" 2>/dev/null | tr -d ' ')"
    if [ -n "$sid" ] && [ "$sid" != "0" ]; then printf 'sid-%s' "$sid"; return 0; fi
    printf 'ppid-%s' "${PPID:-unknown}"
}

slugify() { printf '%s' "$1" | tr -c 'A-Za-z0-9_.-' '-'; }

TOKEN="$(session_token)"
LOCK_FILE="$LOCK_DIR/autospec-run-session-$(slugify "$TOKEN").lock"

cmd="${1:-status}"
shift 2>/dev/null || true
REPO=""
FORCE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:-}"; shift 2 ;;
        --force) FORCE=1; shift ;;
        *) shift ;;
    esac
done

case "$cmd" in
    acquire)
        mkdir -p "$LOCK_DIR"
        [ "$FORCE" = "1" ] && rm -f "$LOCK_FILE"
        # Atomic create: noclobber redirection fails if the file already exists.
        if ( set -o noclobber; : > "$LOCK_FILE" ) 2>/dev/null; then
            printf '{"session":"%s","pid":%s,"repo":"%s","host":"%s","started_at":"%s"}\n' \
                "$TOKEN" "$$" "$REPO" "$(hostname 2>/dev/null || echo unknown)" \
                "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$LOCK_FILE"
            printf 'autospec-run: session lock acquired (session=%s)\n' "$TOKEN"
            exit 0
        fi
        # Held for THIS session already -> refuse + report (per-session single instance).
        existing="$(cat "$LOCK_FILE" 2>/dev/null || true)"
        pid="$(printf '%s' "$existing" | sed -n 's/.*"pid":\([0-9]*\).*/\1/p')"
        alive="dead"
        { [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; } && alive="alive"
        {
            printf 'autospec-run: a monitor is already active for this session (session=%s).\n' "$TOKEN"
            printf '  %s\n' "$existing"
            printf '  holder pid=%s (%s). Run /autospec-stop to halt it, or re-run with --force to override.\n' "${pid:-?}" "$alive"
        } >&2
        exit 3
        ;;
    release)
        rm -f "$LOCK_FILE"
        printf 'autospec-run: session lock released (session=%s)\n' "$TOKEN"
        exit 0
        ;;
    status)
        if [ -f "$LOCK_FILE" ]; then
            cat "$LOCK_FILE"
        else
            printf '{"session":"%s","active":false}\n' "$TOKEN"
        fi
        exit 0
        ;;
    *)
        printf 'usage: autospec-run-session-lock.sh {acquire|release|status} [--repo owner/repo] [--force]\n' >&2
        exit 2
        ;;
esac
