#!/usr/bin/env bash
# scripts/autonomous-spend-ledger.sh — persistent cumulative token/issue tally
# for the /autospec-autonomous perpetual loop.
#
# Ledger path: ~/.autospec/autonomous-spend/<repo-slug>/spend.json
# Path-scoped per repo-slug to prevent cross-repo collisions
# (feedback_heartbeat_cross_repo_collision).
#
# AUTOSPEC_SPEND_SCOPE overrides the per-repo slug with a fixed directory
# name, so a fleet of repos driven off one board can share a single ledger
# (and therefore one lifetime budget) instead of multiplying it per repo.
# Unset (the default) is byte-identical to legacy per-repo behavior. When
# set, the value becomes a ledger directory name, so it is validated against
# an allowlist charset before use — see validate_scope().
#
# Subcommands:
#   add --tokens N [--issues N] [--filed-issues N] [--budget-issues N] [--repo-dir DIR]
#       Increment the cumulative totals in the ledger. Creates the file if
#       absent. Prints the updated totals as JSON.
#
#   check [--repo-dir DIR]
#       Compare ledger totals against caps. Prints either:
#         continue
#         park <reason>
#       When a cap is hit, also invokes notify.sh (PATH-resolved or found in
#       skills/autospec-shared/scripts/) and writes a resume-context block to
#       the ledger. Exit code is always 0 (the decision is communicated via
#       stdout so callers can capture and branch on it without exit-code
#       gymnastics).
#
#   reset [--repo-dir DIR]
#       Zero out the ledger (useful after a resume or quota reset).
#
#   status [--repo-dir DIR]
#       Print the current ledger JSON (or empty object if absent).
#
# Environment caps:
#   AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  (default: 10000000 = 10M)
#   AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES  (default: 500)
#
# Caps of 0 mean "no cap for that dimension" (i.e., they are disabled).
#
# Atomic writes: temp-file + mv so partial writes never corrupt the ledger.
# set -eu, if/then/fi one-sided conditionals (feedback_bash_set_e_short_circuit).
# No RETURN traps (feedback_bash_return_trap_leak).

set -eu

LEDGER_BASE="${HOME}/.autospec/autonomous-spend"
DEFAULT_LIFETIME_TOKENS=10000000
DEFAULT_LIFETIME_ISSUES=500

LIFETIME_TOKENS="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-$DEFAULT_LIFETIME_TOKENS}"
LIFETIME_ISSUES="${AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES:-$DEFAULT_LIFETIME_ISSUES}"

# A scope becomes a directory name; keep it well under filesystem name
# limits (typically 255 bytes) so a bad value fails with a clear message
# here instead of a confusing ENAMETOOLONG from mkdir/mv later.
SCOPE_MAX_LEN=200

# A lock is only reclaimed once BOTH (a) its recorded owner PID is
# provably gone and (b) it has aged past this threshold — see
# ledger_lock_is_stale(). Overridable so tests don't have to sleep for
# the production default.
LOCK_STALE_AGE_SECONDS="${AUTOSPEC_SPEND_LOCK_STALE_SECONDS:-30}"
case "$LOCK_STALE_AGE_SECONDS" in *[!0-9]*|'') LOCK_STALE_AGE_SECONDS=30 ;; esac

# Max polls (at 0.05s apiece, ~10s by default) before ledger_lock_acquire
# gives up on a lock that never becomes free or reclaimable.
LOCK_MAX_WAIT_ITER="${AUTOSPEC_SPEND_LOCK_MAX_WAIT_ITER:-200}"
case "$LOCK_MAX_WAIT_ITER" in *[!0-9]*|'') LOCK_MAX_WAIT_ITER=200 ;; esac

# ── Helpers ──────────────────────────────────────────────────────────────────

die() {
    printf 'autonomous-spend-ledger: %s\n' "$*" >&2
    exit 1
}

info() {
    printf '[autonomous-spend-ledger] %s\n' "$*"
}

# Diagnostics that must never land on stdout (stdout carries machine-read
# JSON/decision text for add/check/status) go to stderr instead.
warn() {
    printf '[autonomous-spend-ledger] %s\n' "$*" >&2
}

require_jq() {
    command -v jq >/dev/null 2>&1 || die "jq is required"
}

iso_now() {
    date -u +'%Y-%m-%dT%H:%M:%SZ'
}

# Derive repo slug from repo origin URL or repo dir path.
# Pattern mirrors autospec-run-registry.sh + autospec-watchdog.sh.
resolve_repo_slug() {
    local repo_dir="${1:-$(pwd)}"
    local slug=""
    slug="$(cd "$repo_dir" 2>/dev/null \
        && git remote get-url origin 2>/dev/null \
        | sed -E 's#^git@[^:]+:##; s#^https?://[^/]+/##; s#\\.git$##; s#[/]#_#g' \
        || true)"
    if [ -z "$slug" ]; then
        # Fallback: sanitize the directory path.
        slug="$(printf '%s' "$repo_dir" | sed 's#[^A-Za-z0-9._-]#_#g')"
    fi
    printf '%s' "$slug"
}

# A scope becomes a directory name verbatim, so validate against an
# allowlist charset (never a denylist) before it ever touches the
# filesystem. Rejects empty, ".", "..", a leading "-" (a foot-gun for any
# later command that takes the scope as a bare argument), anything outside
# [A-Za-z0-9._-] (which also rules out path separators, newlines, and other
# control characters), and anything over SCOPE_MAX_LEN. Sets
# SCOPE_REJECT_REASON on failure so the caller can report a clear cause.
SCOPE_REJECT_REASON=""
validate_scope() {
    local s="$1"
    if [ -z "$s" ]; then
        SCOPE_REJECT_REASON="empty"
        return 1
    fi
    case "$s" in
        .|..)
            SCOPE_REJECT_REASON="'.' and '..' are reserved"
            return 1
            ;;
        -*)
            SCOPE_REJECT_REASON="must not start with '-'"
            return 1
            ;;
    esac
    case "$s" in
        *[!A-Za-z0-9._-]*)
            SCOPE_REJECT_REASON="only [A-Za-z0-9._-] is allowed"
            return 1
            ;;
    esac
    if [ "${#s}" -gt "$SCOPE_MAX_LEN" ]; then
        SCOPE_REJECT_REASON="longer than ${SCOPE_MAX_LEN} characters"
        return 1
    fi
    return 0
}

# resolve_scope prefers AUTOSPEC_SPEND_SCOPE (already validated by the
# caller) over the per-repo slug. Using ${VAR+set} rather than ${VAR:-}
# distinguishes "unset" (legacy per-repo behavior) from "set but empty"
# (an explicit override attempt, rejected by validate_scope upstream).
resolve_scope() {
    local repo_dir="${1:-$(pwd)}"
    if [ "${AUTOSPEC_SPEND_SCOPE+set}" = "set" ]; then
        printf '%s' "${AUTOSPEC_SPEND_SCOPE}"
        return 0
    fi
    resolve_repo_slug "$repo_dir"
}

ledger_path() {
    local repo_dir="${1:-$(pwd)}"
    local slug
    slug="$(resolve_scope "$repo_dir")"
    printf '%s/%s/spend.json' "$LEDGER_BASE" "$slug"
}

# Atomic write: write to a unique per-call temp file in the same dir, then
# mv. mktemp's XXXXXX suffix is randomized per invocation, so concurrent
# writers never share a temp path (that failure mode — a FIXED temp path
# shared by concurrent writers, where the mv is atomic but the source path
# collides — was a Critical finding elsewhere in this feature tonight).
# That said, mv atomicity alone does not make the read-modify-write in
# `add`/`check` atomic: two writers can both read the same ledger, compute
# their own increment, and the second mv silently discards the first's
# update. ledger_lock_acquire/release below close that gap by serializing
# the whole read-modify-write critical section per ledger path — which
# matters more now that a shared AUTOSPEC_SPEND_SCOPE puts multiple
# concurrent workers on the same ledger file instead of one each.
write_json_atomic() {
    local target="$1"
    mkdir -p "$(dirname "$target")"
    local tmp
    tmp="$(mktemp "${target}.XXXXXX")"
    cat > "$tmp"
    mv "$tmp" "$target"
}

# The lock is a symlink, not a plain directory: `ln -s <pid> <lockdir>`
# both claims the lock AND records its owner in one atomic syscall. A
# two-step "mkdir, then separately write a pid file" design leaves a real
# TOCTOU window between the two steps — under heavy contention (many
# workers spinning) that window can stretch far enough for another worker
# to see "no pid recorded yet" and mistake a live, just-acquired lock for
# orphaned. Embedding the pid directly as the symlink target closes that
# window entirely: no other process can ever observe the lock before its
# owner is recorded, because the two never exist independently.
#
# A held lock survives its owning process being SIGKILLed/OOM-killed — the
# EXIT trap in the subcommand bodies below only fires on clean exits. Left
# alone that wedges every future caller on this ledger forever (the ~10s
# timeout just makes every call fail forever, not once). ledger_lock_is_stale
# + ledger_lock_reclaim below reclaim such an orphaned lock safely:
#   - a lock is only ever considered stale when its recorded PID is
#     provably dead (kill -0 fails) AND the lock is older than
#     LOCK_STALE_AGE_SECONDS (belt-and-suspenders against PID reuse) — this
#     can never reclaim a live holder's lock, however old, because kill -0
#     on a live PID always returns success;
#   - a lock whose target isn't a bare PID (unrecognized/foreign) is never
#     auto-reclaimed;
#   - reclaiming itself is serialized through a second mkdir-based mutex
#     ("<lockdir>.reclaiming") so at most one process is ever mid-reclaim
#     for a given lock at a time. This matters even though the final `mv`
#     is atomic: "read the current owner, decide it's stale, then mv" is
#     three separate steps, not one. Under heavy contention, two processes
#     can both read the same stale owner and both decide to reclaim before
#     either mv's; the first to mv wins and immediately re-acquires the
#     lock for itself, and — without the mutex — the second's *already
#     stale-approved* mv would still fire moments later and blindly steal
#     whatever is now at that path, even though it's a brand new, live
#     lock. Gating the whole read-decide-mv sequence behind one mutex
#     means only the winner ever gets far enough to call mv, so a late
#     second reclaimer just finds the mutex held and backs off instead of
#     re-verifying and racing anyway. (Verified empirically: without this
#     mutex, tight 8-way contention against a single pre-staged stale lock
#     reproducibly stole a live sibling's lock and lost an update; ~100
#     runs with the mutex in place saw zero.)
ledger_lock_is_stale() {
    local lockdir="$1"
    local pid
    pid="$(readlink "$lockdir" 2>/dev/null || true)"
    case "$pid" in
        *[!0-9]*|'')
            # Not a lock we recognize (foreign object, or a race where the
            # symlink vanished between the failed ln and this check) —
            # never auto-reclaim something we don't understand.
            return 1
            ;;
    esac
    if kill -0 "$pid" 2>/dev/null; then
        return 1
    fi
    if ledger_lock_is_old_enough "$lockdir"; then
        STALE_LOCK_PID="$pid"
        return 0
    fi
    return 1
}

ledger_lock_is_old_enough() {
    local lockdir="$1"
    local mtime now age
    mtime="$( (stat -f %m "$lockdir" 2>/dev/null || stat -c %Y "$lockdir" 2>/dev/null) || true)"
    case "$mtime" in *[!0-9]*|'') return 1 ;; esac
    now="$(date +%s)"
    age=$((now - mtime))
    if [ "$age" -ge "$LOCK_STALE_AGE_SECONDS" ]; then
        return 0
    fi
    return 1
}

# Set by ledger_lock_is_stale on a positive staleness verdict, to the exact
# PID that verdict was based on. ledger_lock_reclaim re-verifies against
# this value (not a fresh staleness recomputation) so it only ever acts on
# the specific state it was authorized for.
STALE_LOCK_PID=""

# Attempt to reclaim a stale lock. Only one process at a time gets past the
# "${lockdir}.reclaiming" mutex (mkdir is atomic/exclusive), so the
# read-current -> compare -> mv sequence below never runs concurrently with
# itself for a given lock — see the block comment above for why that
# matters beyond the atomicity of `mv` alone.
ledger_lock_reclaim() {
    local lockdir="$1"
    local expected="$STALE_LOCK_PID"
    local reclaim_mutex="${lockdir}.reclaiming"
    if ! mkdir "$reclaim_mutex" 2>/dev/null; then
        # Someone else is already reclaiming this lock; let them finish
        # and fall through to retry in the normal wait loop.
        return 0
    fi
    # Test-only seam: unset/empty in production (zero cost, no behavior
    # change). Lets tests deterministically widen the window between
    # winning the reclaim mutex and re-reading the current target, to
    # prove the mismatch-abort branch below (the lock changed hands after
    # we decided it was stale) is exercised and still bounded — not to
    # simulate any real production condition.
    if [ -n "${AUTOSPEC_SPEND_LOCK_TEST_STALL:-}" ]; then
        sleep "$AUTOSPEC_SPEND_LOCK_TEST_STALL"
    fi
    local current
    current="$(readlink "$lockdir" 2>/dev/null || true)"
    if [ "$current" = "$expected" ]; then
        local graveyard="${lockdir}.stale.$$"
        if mv "$lockdir" "$graveyard" 2>/dev/null; then
            warn "reclaimed a stale ledger lock (owner process gone): $lockdir"
            rm -f "$graveyard"
        fi
    fi
    rmdir "$reclaim_mutex" 2>/dev/null || true
}

ledger_lock_acquire() {
    local ledger="$1"
    local lockdir="${ledger}.lock"
    mkdir -p "$(dirname "$ledger")"
    local waited=0
    while :; do
        if ln -s "$$" "$lockdir" 2>/dev/null; then
            return 0
        fi
        # Attempting a reclaim (successful, a no-op because the reclaim
        # mutex is held/orphaned, or aborted because the target changed)
        # must NEVER skip the wait/timeout accounting below via `continue`
        # — doing that turned an orphaned "<lockdir>.reclaiming" mutex into
        # an unbounded, silent busy-spin (no sleep, no wait counter, never
        # reaches LOCK_MAX_WAIT_ITER). Every iteration of this loop, no
        # matter what happened above, falls through to the same bounded
        # wait/timeout step, so every path out of this function is either
        # "lock acquired" or "die after LOCK_MAX_WAIT_ITER retries" — never
        # an infinite loop.
        if ledger_lock_is_stale "$lockdir"; then
            ledger_lock_reclaim "$lockdir"
        fi
        waited=$((waited + 1))
        if [ "$waited" -gt "$LOCK_MAX_WAIT_ITER" ]; then
            die "timed out waiting for ledger lock: $lockdir"
        fi
        sleep 0.05
    done
}

ledger_lock_release() {
    local ledger="$1"
    rm -f "${ledger}.lock" 2>/dev/null || true
}

read_ledger() {
    local path="$1"
    if [ -f "$path" ]; then
        cat "$path"
    else
        # Return a zero-state JSON if absent.
        printf '{"schema":1,"tokens":0,"issues":0,"filed_issues":0,"budget_issues":0,"created_at":"%s","updated_at":"%s","parked":false}\n' \
            "$(iso_now)" "$(iso_now)"
    fi
}

# Find the notify.sh helper: check PATH first, then the skill scripts location.
find_notify() {
    if command -v notify.sh >/dev/null 2>&1; then
        printf 'notify.sh'
        return
    fi
    local repo_dir="${1:-$(pwd)}"
    local skill_path="${repo_dir}/skills/autospec-shared/scripts/notify.sh"
    if [ -f "$skill_path" ]; then
        printf '%s' "$skill_path"
        return
    fi
    # Not found — caller degrades gracefully.
    printf ''
}

# ── Subcommand parsing ───────────────────────────────────────────────────────

usage() {
    cat <<'EOF'
Usage: autonomous-spend-ledger.sh <subcommand> [options]

Subcommands:
  add     --tokens N [--issues N] [--filed-issues N] [--budget-issues N] [--repo-dir DIR]
  check   [--repo-dir DIR]
  reset   [--repo-dir DIR]
  status  [--repo-dir DIR]

Env:
  AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  (default 10000000)
  AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES  (default 500)

Exit code is always 0; decision output is on stdout.
EOF
}

SUBCMD="${1:-}"
[ -n "$SUBCMD" ] || { usage >&2; exit 1; }
case "$SUBCMD" in --help|-h) usage; exit 0 ;; esac
shift

ADD_TOKENS=0
ADD_ISSUES=0
ADD_FILED_ISSUES=""
ADD_BUDGET_ISSUES=""
REPO_DIR="$(pwd)"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --tokens)        ADD_TOKENS="${2:-0}"; shift 2 ;;
        --issues)        ADD_ISSUES="${2:-0}"; shift 2 ;;
        --filed-issues)  ADD_FILED_ISSUES="${2:-0}"; shift 2 ;;
        --budget-issues) ADD_BUDGET_ISSUES="${2:-0}"; shift 2 ;;
        --repo-dir)      REPO_DIR="${2:-$(pwd)}"; shift 2 ;;
        --help|-h)       usage; exit 0 ;;
        *)               die "unknown option: $1" ;;
    esac
done

# Validate AUTOSPEC_SPEND_SCOPE at top level (not inside a command
# substitution) so `die`'s exit actually terminates this process rather
# than just the subshell that would otherwise swallow it.
if [ "${AUTOSPEC_SPEND_SCOPE+set}" = "set" ]; then
    if ! validate_scope "${AUTOSPEC_SPEND_SCOPE}"; then
        SCOPE_DISPLAY="${AUTOSPEC_SPEND_SCOPE}"
        if [ "${#SCOPE_DISPLAY}" -gt 40 ]; then
            SCOPE_DISPLAY="${SCOPE_DISPLAY:0:40}...(truncated, ${#AUTOSPEC_SPEND_SCOPE} chars total)"
        fi
        die "invalid AUTOSPEC_SPEND_SCOPE (${SCOPE_REJECT_REASON}): ${SCOPE_DISPLAY}"
    fi
fi

LEDGER="$(ledger_path "$REPO_DIR")"

# ── Subcommand: add ──────────────────────────────────────────────────────────
if [ "$SUBCMD" = "add" ]; then
    require_jq
    case "$ADD_TOKENS" in *[!0-9]*|'') die "--tokens must be a non-negative integer" ;; esac
    case "$ADD_ISSUES" in *[!0-9]*|'') die "--issues must be a non-negative integer" ;; esac
    if [ -z "$ADD_FILED_ISSUES" ]; then
        ADD_FILED_ISSUES="$ADD_ISSUES"
    fi
    if [ -z "$ADD_BUDGET_ISSUES" ]; then
        ADD_BUDGET_ISSUES="$ADD_ISSUES"
    fi
    case "$ADD_FILED_ISSUES" in *[!0-9]*|'') die "--filed-issues must be a non-negative integer" ;; esac
    case "$ADD_BUDGET_ISSUES" in *[!0-9]*|'') die "--budget-issues must be a non-negative integer" ;; esac

    ledger_lock_acquire "$LEDGER"
    trap 'ledger_lock_release "$LEDGER"' EXIT

    current="$(read_ledger "$LEDGER")"
    updated="$(printf '%s' "$current" | jq \
        --argjson t "$ADD_TOKENS" \
        --argjson filed "$ADD_FILED_ISSUES" \
        --argjson budget "$ADD_BUDGET_ISSUES" \
        --arg ts "$(iso_now)" \
        '.tokens = ((.tokens // 0) + $t)
         | .filed_issues = ((.filed_issues // .issues // 0) + $filed)
         | .budget_issues = ((.budget_issues // .issues // 0) + $budget)
         | .issues = .budget_issues
         | .updated_at = $ts')"
    printf '%s\n' "$updated" | write_json_atomic "$LEDGER"

    ledger_lock_release "$LEDGER"
    trap - EXIT
    printf '%s\n' "$updated"
    exit 0
fi

# ── Subcommand: check ────────────────────────────────────────────────────────
if [ "$SUBCMD" = "check" ]; then
    require_jq
    ledger_lock_acquire "$LEDGER"
    trap 'ledger_lock_release "$LEDGER"' EXIT
    current="$(read_ledger "$LEDGER")"
    total_tokens="$(printf '%s' "$current" | jq -r '.tokens // 0')"
    total_issues="$(printf '%s' "$current" | jq -r '.budget_issues // .issues // 0')"

    # Validate that the values are integers before arithmetic comparison.
    case "$total_tokens" in *[!0-9]*|'') total_tokens=0 ;; esac
    case "$total_issues" in *[!0-9]*|'') total_issues=0 ;; esac
    case "$LIFETIME_TOKENS" in *[!0-9]*|'') LIFETIME_TOKENS="$DEFAULT_LIFETIME_TOKENS" ;; esac
    case "$LIFETIME_ISSUES" in *[!0-9]*|'') LIFETIME_ISSUES="$DEFAULT_LIFETIME_ISSUES" ;; esac

    park_reason=""

    # Token cap: 0 means disabled.
    if [ "$LIFETIME_TOKENS" -gt 0 ] && [ "$total_tokens" -ge "$LIFETIME_TOKENS" ]; then
        park_reason="lifetime token cap reached (${total_tokens} >= ${LIFETIME_TOKENS})"
    fi

    # Issue cap: 0 means disabled.
    if [ -z "$park_reason" ] && [ "$LIFETIME_ISSUES" -gt 0 ] && [ "$total_issues" -ge "$LIFETIME_ISSUES" ]; then
        park_reason="lifetime issue cap reached (${total_issues} >= ${LIFETIME_ISSUES})"
    fi

    if [ -n "$park_reason" ]; then
        # Write parked state + resume context to ledger.
        parked_ledger="$(printf '%s' "$current" | jq \
            --arg reason "$park_reason" \
            --arg ts "$(iso_now)" \
            '.budget_issues = (.budget_issues // .issues // 0)
             | .filed_issues = (.filed_issues // .issues // 0)
             | .issues = .budget_issues
             | .parked = true
             | .park_reason = $reason
             | .parked_at = $ts')"
        printf '%s\n' "$parked_ledger" | write_json_atomic "$LEDGER"

        # Invoke notify.sh (fail-open: notifier errors must never block).
        notifier="$(find_notify "$REPO_DIR")"
        if [ -n "$notifier" ]; then
            bash "$notifier" "autospec-autonomous parked" "$park_reason" || true
        fi

        printf 'park %s\n' "$park_reason"
    else
        printf 'continue\n'
    fi
    exit 0
fi

# ── Subcommand: reset ────────────────────────────────────────────────────────
if [ "$SUBCMD" = "reset" ]; then
    require_jq
    ledger_lock_acquire "$LEDGER"
    trap 'ledger_lock_release "$LEDGER"' EXIT
    jq -n \
        --arg ts "$(iso_now)" \
        '{"schema":1,"tokens":0,"issues":0,"filed_issues":0,"budget_issues":0,"created_at":$ts,"updated_at":$ts,"parked":false}' \
        | write_json_atomic "$LEDGER"
    info "ledger reset: $LEDGER"
    exit 0
fi

# ── Subcommand: status ───────────────────────────────────────────────────────
if [ "$SUBCMD" = "status" ]; then
    require_jq
    read_ledger "$LEDGER" | jq \
        '.budget_issues = (.budget_issues // .issues // 0)
         | .filed_issues = (.filed_issues // .issues // 0)
         | .issues = .budget_issues'
    exit 0
fi

# ── Unknown subcommand ────────────────────────────────────────────────────────
usage >&2
exit 1
