#!/usr/bin/env bash
# scripts/autospec-repo-lock.sh — optional repo-scoped advisory lock
#
# Provides acquire/release keyed by canonical repo slug (owner__name form
# produced by scripts/repo-slug.sh).  Default: completely OFF.  Opt-in:
#   export AUTOSPEC_REPO_LOCK=1
#
# Lock store: ${AUTOSPEC_REPO_LOCK_DIR:-~/.autospec/repo-locks}/<slug>.lock/
#   The lock "directory" is created atomically via `mkdir`; a "pid" file
#   inside records the owner PID so stale locks from dead processes can be
#   detected and broken.
#
# Cross-host workers do NOT share this lock dir → they are never blocked.
# Same-machine monitors sharing one checkout DO share it → contention is
# serialised via the bounded spin loop.
#
# Reusing scripts/repo-slug.sh: canonical_slug() is the same function that
# all other helpers (heartbeat-write, watchdog) use; no fork of slug logic.
#
# Usage (source or standalone):
#   bash scripts/autospec-repo-lock.sh acquire <slug>   # exits 0 on success
#   bash scripts/autospec-repo-lock.sh release <slug>   # exits 0 always
#
# Environment:
#   AUTOSPEC_REPO_LOCK          set to "1" to enable; unset/empty → no-op
#   AUTOSPEC_REPO_LOCK_DIR      override default lock-store path
#   AUTOSPEC_REPO_LOCK_TIMEOUT  max seconds to wait (default 30)
#   AUTOSPEC_REPO_LOCK_POLL     poll interval in seconds (default 2)

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
AUTOSPEC_REPO_LOCK_DIR="${AUTOSPEC_REPO_LOCK_DIR:-${HOME}/.autospec/repo-locks}"
AUTOSPEC_REPO_LOCK_TIMEOUT="${AUTOSPEC_REPO_LOCK_TIMEOUT:-30}"
AUTOSPEC_REPO_LOCK_POLL="${AUTOSPEC_REPO_LOCK_POLL:-2}"

# ---------------------------------------------------------------------------
# _lock_dir <slug> → path of the lock directory for this slug
# ---------------------------------------------------------------------------
_lock_dir() {
    local slug="${1:?_lock_dir: slug required}"
    printf '%s/%s.lock' "$AUTOSPEC_REPO_LOCK_DIR" "$slug"
}

# ---------------------------------------------------------------------------
# _is_pid_alive <pid> → 0 if the process is running, 1 otherwise
# ---------------------------------------------------------------------------
_is_pid_alive() {
    local pid="${1:-}"
    [ -n "$pid" ] || return 1
    # kill -0 checks existence without sending a signal
    kill -0 "$pid" 2>/dev/null
}

# ---------------------------------------------------------------------------
# _break_stale_lock <lock_dir>
# Removes the lock directory if the recorded PID is no longer running.
# ---------------------------------------------------------------------------
_break_stale_lock() {
    local ldir="$1"
    local pid_file="${ldir}/pid"
    if [ ! -d "$ldir" ]; then
        return 0   # already gone
    fi
    if [ ! -f "$pid_file" ]; then
        # Malformed lock dir — remove it
        rm -rf "$ldir" 2>/dev/null || true
        return 0
    fi
    local owner_pid
    owner_pid="$(cat "$pid_file" 2>/dev/null || true)"
    if ! _is_pid_alive "$owner_pid"; then
        # Stale lock: owner process is dead
        rm -rf "$ldir" 2>/dev/null || true
    fi
}

# ---------------------------------------------------------------------------
# repo_lock_acquire <slug>
# Acquires the advisory lock.  Returns 0 on success, 1 on timeout.
# ---------------------------------------------------------------------------
repo_lock_acquire() {
    local slug="${1:?repo_lock_acquire: slug required}"

    # Fast path: opt-in not set → no-op
    if [ "${AUTOSPEC_REPO_LOCK:-}" != "1" ]; then
        return 0
    fi

    local ldir
    ldir="$(_lock_dir "$slug")"
    mkdir -p "$AUTOSPEC_REPO_LOCK_DIR"

    local deadline=$(( $(date +%s) + AUTOSPEC_REPO_LOCK_TIMEOUT ))
    local my_pid="$$"

    while true; do
        # Try to break a stale lock before attempting mkdir
        if [ -d "$ldir" ]; then
            _break_stale_lock "$ldir"
        fi

        # Atomic mkdir — succeeds only for one caller
        if mkdir "$ldir" 2>/dev/null; then
            printf '%s\n' "$my_pid" > "${ldir}/pid"
            return 0
        fi

        # Check for timeout
        local now
        now="$(date +%s)"
        if [ "$now" -ge "$deadline" ]; then
            printf 'autospec-repo-lock: acquire timed out after %s seconds (slug=%s, lock=%s)\n' \
                "$AUTOSPEC_REPO_LOCK_TIMEOUT" "$slug" "$ldir" >&2
            return 1
        fi

        sleep "$AUTOSPEC_REPO_LOCK_POLL"
    done
}

# ---------------------------------------------------------------------------
# repo_lock_release <slug>
# Releases the advisory lock.  Safe to call when not held; always returns 0.
# ---------------------------------------------------------------------------
repo_lock_release() {
    local slug="${1:?repo_lock_release: slug required}"

    # Fast path: opt-in not set → no-op
    if [ "${AUTOSPEC_REPO_LOCK:-}" != "1" ]; then
        return 0
    fi

    local ldir
    ldir="$(_lock_dir "$slug")"

    if [ ! -d "$ldir" ]; then
        # Already released or never held — silent success
        return 0
    fi

    local pid_file="${ldir}/pid"
    if [ -f "$pid_file" ]; then
        local owner_pid
        owner_pid="$(cat "$pid_file" 2>/dev/null || true)"
        # Only refuse if a *different, live* process owns the lock.
        # If the owner PID is dead (e.g. standalone acquire exited, or crashed),
        # treat it as a stale lock and allow the release.
        if [ "$owner_pid" != "$$" ] && _is_pid_alive "$owner_pid"; then
            printf 'autospec-repo-lock: release skipped; lock owned by live pid %s, we are %s (slug=%s)\n' \
                "$owner_pid" "$$" "$slug" >&2
            return 0
        fi
    fi

    rm -rf "$ldir" 2>/dev/null || true
    return 0
}

# ---------------------------------------------------------------------------
# Standalone entrypoint
# ---------------------------------------------------------------------------
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    cmd="${1:-}"
    slug="${2:-}"
    if [ -z "$slug" ]; then
        printf 'Usage: %s acquire|release <slug>\n' "$0" >&2
        exit 1
    fi
    case "$cmd" in
        acquire) repo_lock_acquire "$slug" ;;
        release) repo_lock_release "$slug" ;;
        *)
            printf 'autospec-repo-lock: unknown command "%s" (expected acquire|release)\n' "$cmd" >&2
            exit 1
            ;;
    esac
fi
