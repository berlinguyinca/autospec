#!/usr/bin/env bash
# scripts/autonomous-resilience.sh — conductor resilience decision helpers.
#
# Provides four subcommands to support the /autospec-autonomous conductor's
# long-run health without adding a second lock or contradicting autospec-resume's
# existing 300s/10800s staleness model (feedback_heartbeat_cross_repo_collision,
# feedback_bash_set_e_short_circuit, feedback_bash_return_trap_leak).
#
# Subcommands:
#   state  write  --repo OWNER/REPO --status STATUS [--session SESSION]
#   state  read   --repo OWNER/REPO
#   lock   acquire --repo OWNER/REPO [--session SESSION]
#   lock   release --repo OWNER/REPO [--session SESSION]
#   lock   check   --repo OWNER/REPO
#   quarantine  --repo OWNER/REPO --issue N [--failures N]
#   main-health --repo OWNER/REPO
#
# Machine-readable output on stdout — one DECISION:<token> line per call:
#   DECISION:state-written
#   DECISION:lock-acquired   (also: LOCK_SESSION:<id>)
#   DECISION:lock-held       (exit 1)
#   DECISION:lock-released
#   DECISION:lock-available  /  DECISION:lock-held
#   DECISION:quarantine      (exit 1) — issue labeled autospec:needs-human
#   DECISION:continue        — failure count < cap
#   DECISION:continue        — main status green
#   DECISION:wait            — main status pending
#   DECISION:halt            — main status red (exit 1)
#
# Single-instance lock reconciled with autospec-resume (NOT a second lock):
#   Reclaimable ONLY when holder heartbeat_at satisfies
#     (now - heartbeat_at >= RECLAIM_SECS=10800)
#   OR (status=claimed AND now - heartbeat_at >= CLAIMED_TIMEOUT=300).
#   This is the SAME staleness logic as resume-scan.sh:86-105 so two concurrent
#   resumes + the conductor converge to exactly one claim — never double-run.
#   Explicit handoff: `lock release` clears the lock; resume inherits cleanly.
#
# Path scoping:
#   Uses scripts/repo-slug.sh canonical_slug() → owner__name (double underscore)
#   to scope all state under ~/.autospec/autonomous/<slug>/ and avoid the
#   cross-repo heartbeat collision (feedback_heartbeat_cross_repo_collision).
#
# Environment overrides (testing):
#   AUTOSPEC_STATE_DIR           base dir (default: ~/.autospec)
#   AUTOSPEC_GH_CMD              gh binary (default: gh from PATH)
#   AUTOSPEC_NOTIFY_SH           notify.sh path override
#   AUTOSPEC_HOST                hostname override
#   AUTOSPEC_SESSION_ID          session-id override
#   AUTOSPEC_ISSUE_FAILURE_CAP   per-issue failure cap (default: 3)
#   WATCHDOG_CLAIMED_TIMEOUT_SECS  (default: 300)
#   WATCHDOG_RECLAIM_SECS          (default: 10800)
#   AUTOSPEC_REPO                  repo override (owner/repo)
#
# Exit codes:
#   0  — success / decision emitted
#   1  — lock-held / quarantine-active / halt
#   2  — usage error

set -eu

# ── Constants ─────────────────────────────────────────────────────────────────
CLAIMED_TIMEOUT="${WATCHDOG_CLAIMED_TIMEOUT_SECS:-300}"
RECLAIM_SECS="${WATCHDOG_RECLAIM_SECS:-10800}"
FAILURE_CAP="${AUTOSPEC_ISSUE_FAILURE_CAP:-3}"
STATE_BASE="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/autonomous"
GH="${AUTOSPEC_GH_CMD:-gh}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Helpers ───────────────────────────────────────────────────────────────────
die() { printf 'autonomous-resilience: %s\n' "$1" >&2; exit 2; }
say() { printf '%s\n' "$1"; }

now_ts() { date -u +%s; }

# Resolve canonical slug via repo-slug.sh.
# Falls back to a simple tr-based form when the helper is absent.
canonical_slug() {
    local repo="$1"
    local helper=""
    if [ -f "$SCRIPT_DIR/repo-slug.sh" ]; then
        helper="$SCRIPT_DIR/repo-slug.sh"
    elif [ -f "${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/scripts/repo-slug.sh" ]; then
        helper="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/scripts/repo-slug.sh"
    fi
    if [ -n "$helper" ]; then
        bash "$helper" --canonical "$repo" 2>/dev/null || printf '%s' "$repo" | sed 's#/#__#'
    else
        printf '%s' "$repo" | sed 's#/#__#'
    fi
}

# Derive repo from git remote when not explicitly provided.
resolve_repo() {
    local repo="${1:-}"
    if [ -z "$repo" ]; then
        repo="${AUTOSPEC_REPO:-}"
    fi
    if [ -z "$repo" ]; then
        repo="$("$GH" repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || echo "")"
    fi
    if [ -z "$repo" ]; then
        die "--repo OWNER/REPO is required (or set AUTOSPEC_REPO)"
    fi
    printf '%s' "$repo"
}

# State dir for a repo: ~/.autospec/autonomous/<slug>/
state_dir() {
    local repo="$1"
    local slug
    slug="$(canonical_slug "$repo")"
    printf '%s/%s' "$STATE_BASE" "$slug"
}

# Read the state.json file; emit empty object if absent.
read_state_json() {
    local dir="$1"
    local state_file="$dir/state.json"
    if [ -f "$state_file" ]; then
        cat "$state_file"
    else
        printf '{}'
    fi
}

# Write state.json atomically via temp+mv.
write_state_json() {
    local dir="$1"
    local json="$2"
    mkdir -p "$dir"
    local tmp
    tmp="$(mktemp "$dir/state.json.XXXXXX")"
    printf '%s\n' "$json" > "$tmp"
    mv "$tmp" "$dir/state.json"
}

# Notify helper — always exit 0 (notifier failures must not block the conductor).
notify_op() {
    local title="$1"
    local body="$2"
    local notify_sh=""
    if [ -n "${AUTOSPEC_NOTIFY_SH:-}" ] && [ -f "$AUTOSPEC_NOTIFY_SH" ]; then
        notify_sh="$AUTOSPEC_NOTIFY_SH"
    elif [ -f "$SCRIPT_DIR/../skills/autospec-shared/scripts/notify.sh" ]; then
        notify_sh="$SCRIPT_DIR/../skills/autospec-shared/scripts/notify.sh"
    elif [ -f "${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/scripts/notify.sh" ]; then
        notify_sh="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/scripts/notify.sh"
    fi
    if [ -n "$notify_sh" ]; then
        bash "$notify_sh" "$title" "$body" || true
    else
        printf 'notify: %s — %s\n' "$title" "$body" || true
    fi
}

# ── Subcommand: state ─────────────────────────────────────────────────────────
# state write --repo OWNER/REPO --status STATUS [--session SESSION]
# state read  --repo OWNER/REPO
cmd_state() {
    local subcmd="${1:-}"; shift || true
    case "$subcmd" in
        write) _state_write "$@" ;;
        read)  _state_read  "$@" ;;
        *)     die "state: unknown subcommand '${subcmd}'. Use write|read." ;;
    esac
}

_state_write() {
    local repo="" status="" session=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)    repo="${2:-}";    shift 2 ;;
            --status)  status="${2:-}";  shift 2 ;;
            --session) session="${2:-}"; shift 2 ;;
            *) die "state write: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"
    if [ -z "$status" ]; then
        die "state write: --status is required"
    fi

    local dir
    dir="$(state_dir "$repo")"
    local slug
    slug="$(canonical_slug "$repo")"
    local host="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"
    local sess="${AUTOSPEC_SESSION_ID:-${CLAUDE_CODE_SESSION_ID:-$$}}"
    if [ -n "$session" ]; then
        sess="$session"
    fi
    local ts
    ts="$(now_ts)"

    # Preserve existing lock fields if present.
    local existing
    existing="$(read_state_json "$dir")"
    local lock_pid lock_host lock_session lock_acquired_at
    lock_pid="$(printf '%s' "$existing" | jq -r '.lock_pid // empty' 2>/dev/null || echo "")"
    lock_host="$(printf '%s' "$existing" | jq -r '.lock_host // empty' 2>/dev/null || echo "")"
    lock_session="$(printf '%s' "$existing" | jq -r '.lock_session // empty' 2>/dev/null || echo "")"
    lock_acquired_at="$(printf '%s' "$existing" | jq -r '.lock_acquired_at // empty' 2>/dev/null || echo "")"

    local json
    json="$(jq -n \
        --arg repo "$repo" \
        --arg slug "$slug" \
        --arg status "$status" \
        --arg host "$host" \
        --arg session "$sess" \
        --argjson ts "$ts" \
        --arg lock_pid "${lock_pid:-}" \
        --arg lock_host "${lock_host:-}" \
        --arg lock_session "${lock_session:-}" \
        --arg lock_acquired_at "${lock_acquired_at:-}" \
        '{
            repo: $repo,
            slug: $slug,
            status: $status,
            host: $host,
            session: $session,
            heartbeat_at: $ts,
            lock_pid: (if $lock_pid == "" then null else ($lock_pid | tonumber?) // null end),
            lock_host: (if $lock_host == "" then null else $lock_host end),
            lock_session: (if $lock_session == "" then null else $lock_session end),
            lock_acquired_at: (if $lock_acquired_at == "" then null else ($lock_acquired_at | tonumber?) // null end)
        }'
    )"
    write_state_json "$dir" "$json"
    say "DECISION:state-written"
    say "STATUS:${status}"
    say "HEARTBEAT_AT:${ts}"
}

_state_read() {
    local repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "state read: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"
    local dir
    dir="$(state_dir "$repo")"
    read_state_json "$dir"
}

# ── Subcommand: lock ──────────────────────────────────────────────────────────
cmd_lock() {
    local subcmd="${1:-}"; shift || true
    case "$subcmd" in
        acquire) _lock_acquire "$@" ;;
        release) _lock_release "$@" ;;
        check)   _lock_check   "$@" ;;
        *)       die "lock: unknown subcommand '${subcmd}'. Use acquire|release|check." ;;
    esac
}

# Decide if an existing lock is stale per resume's thresholds.
# Outputs "stale" or "live" to stdout.
_lock_staleness() {
    local existing_json="$1"
    local now="$2"

    local heartbeat_at status lock_pid
    heartbeat_at="$(printf '%s' "$existing_json" | jq -r '.heartbeat_at // empty' 2>/dev/null || echo "")"
    status="$(printf '%s' "$existing_json" | jq -r '.status // empty' 2>/dev/null || echo "")"
    lock_pid="$(printf '%s' "$existing_json" | jq -r '.lock_pid // empty' 2>/dev/null || echo "")"

    # No lock → available (not stale, but caller treats absence as available)
    if [ -z "$lock_pid" ]; then
        printf 'no-lock'
        return
    fi

    # No heartbeat recorded → treat as stale (safe recovery path)
    if [ -z "$heartbeat_at" ]; then
        printf 'stale'
        return
    fi

    local age
    age="$(( now - heartbeat_at ))"

    # Mirror resume-scan.sh staleness logic exactly:
    #   step=claimed && age>=CLAIMED_TIMEOUT  →  stale
    #   age>=RECLAIM_SECS                     →  stale (any status)
    if [ "$age" -ge "$RECLAIM_SECS" ]; then
        printf 'stale'
        return
    fi
    if [ "$status" = "claimed" ] && [ "$age" -ge "$CLAIMED_TIMEOUT" ]; then
        printf 'stale'
        return
    fi
    printf 'live'
}

_lock_acquire() {
    local repo="" session=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)    repo="${2:-}";    shift 2 ;;
            --session) session="${2:-}"; shift 2 ;;
            *) die "lock acquire: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"

    local dir
    dir="$(state_dir "$repo")"
    local slug
    slug="$(canonical_slug "$repo")"
    local host="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"
    local sess="${AUTOSPEC_SESSION_ID:-${CLAUDE_CODE_SESSION_ID:-$$}}"
    if [ -n "$session" ]; then
        sess="$session"
    fi
    local ts
    ts="$(now_ts)"

    local existing
    existing="$(read_state_json "$dir")"
    local staleness
    staleness="$(_lock_staleness "$existing" "$ts")"

    if [ "$staleness" = "live" ]; then
        local holder_session holder_host
        holder_session="$(printf '%s' "$existing" | jq -r '.lock_session // empty' 2>/dev/null || echo "")"
        holder_host="$(printf '%s' "$existing" | jq -r '.lock_host // empty' 2>/dev/null || echo "")"
        say "DECISION:lock-held"
        say "HOLDER_SESSION:${holder_session}"
        say "HOLDER_HOST:${holder_host}"
        exit 1
    fi

    # Stale or no-lock → acquire. Preserve existing non-lock state fields.
    local existing_status existing_session existing_heartbeat
    existing_status="$(printf '%s' "$existing" | jq -r '.status // empty' 2>/dev/null || echo "idle")"
    existing_session="$(printf '%s' "$existing" | jq -r '.session // empty' 2>/dev/null || echo "")"
    existing_heartbeat="$(printf '%s' "$existing" | jq -r '.heartbeat_at // empty' 2>/dev/null || echo "0")"

    # Use $$ as pid (the current shell process)
    local pid="$$"

    local json
    json="$(jq -n \
        --arg repo "$repo" \
        --arg slug "$slug" \
        --arg status "${existing_status:-idle}" \
        --arg host "$host" \
        --arg session "$sess" \
        --argjson ts "$ts" \
        --argjson pid "$pid" \
        --arg lock_host "$host" \
        --arg lock_session "$sess" \
        '{
            repo: $repo,
            slug: $slug,
            status: $status,
            host: $host,
            session: $session,
            heartbeat_at: $ts,
            lock_pid: $pid,
            lock_host: $lock_host,
            lock_session: $lock_session,
            lock_acquired_at: $ts
        }'
    )"
    write_state_json "$dir" "$json"
    say "DECISION:lock-acquired"
    say "LOCK_SESSION:${sess}"
    say "LOCK_ACQUIRED_AT:${ts}"
}

_lock_release() {
    local repo="" session=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)    repo="${2:-}";    shift 2 ;;
            --session) session="${2:-}"; shift 2 ;;
            *) die "lock release: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"

    local dir
    dir="$(state_dir "$repo")"
    local ts
    ts="$(now_ts)"

    local existing
    existing="$(read_state_json "$dir")"
    local slug
    slug="$(canonical_slug "$repo")"
    local host="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"
    local existing_status existing_session
    existing_status="$(printf '%s' "$existing" | jq -r '.status // empty' 2>/dev/null || echo "idle")"
    existing_session="$(printf '%s' "$existing" | jq -r '.session // empty' 2>/dev/null || echo "")"

    local json
    json="$(jq -n \
        --arg repo "$repo" \
        --arg slug "$slug" \
        --arg status "${existing_status:-idle}" \
        --arg host "$host" \
        --arg session "${existing_session:-}" \
        --argjson ts "$ts" \
        '{
            repo: $repo,
            slug: $slug,
            status: $status,
            host: $host,
            session: $session,
            heartbeat_at: $ts,
            lock_pid: null,
            lock_host: null,
            lock_session: null,
            lock_acquired_at: null
        }'
    )"
    write_state_json "$dir" "$json"
    say "DECISION:lock-released"
}

_lock_check() {
    local repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "lock check: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"

    local dir
    dir="$(state_dir "$repo")"
    local ts
    ts="$(now_ts)"

    local existing
    existing="$(read_state_json "$dir")"
    local staleness
    staleness="$(_lock_staleness "$existing" "$ts")"

    if [ "$staleness" = "live" ]; then
        local holder_session holder_host
        holder_session="$(printf '%s' "$existing" | jq -r '.lock_session // empty' 2>/dev/null || echo "")"
        holder_host="$(printf '%s' "$existing" | jq -r '.lock_host // empty' 2>/dev/null || echo "")"
        say "DECISION:lock-held"
        say "HOLDER_SESSION:${holder_session}"
        say "HOLDER_HOST:${holder_host}"
        exit 1
    fi
    say "DECISION:lock-available"
    say "STALENESS:${staleness}"
}

# ── Subcommand: quarantine ────────────────────────────────────────────────────
# quarantine --repo OWNER/REPO --issue N [--failures N]
cmd_quarantine() {
    local repo="" issue="" failures=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)     repo="${2:-}";     shift 2 ;;
            --issue)    issue="${2:-}";    shift 2 ;;
            --failures) failures="${2:-}"; shift 2 ;;
            *) die "quarantine: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"
    if [ -z "$issue" ]; then
        die "quarantine: --issue N is required"
    fi

    local dir
    dir="$(state_dir "$repo")"
    local issues_dir="$dir/issues"
    mkdir -p "$issues_dir"

    local issue_file="$issues_dir/${issue}.json"

    # Read existing failure count.
    local current_failures=0
    if [ -f "$issue_file" ]; then
        current_failures="$(jq -r '.failures // 0' "$issue_file" 2>/dev/null || echo 0)"
    fi

    # If --failures explicitly provided, use it; otherwise increment.
    if [ -n "$failures" ]; then
        current_failures="$failures"
    else
        current_failures="$(( current_failures + 1 ))"
    fi

    # Write updated failure count atomically.
    local ts
    ts="$(now_ts)"
    local tmp_issue
    tmp_issue="$(mktemp "$issues_dir/${issue}.json.XXXXXX")"
    jq -n \
        --arg issue "$issue" \
        --argjson failures "$current_failures" \
        --argjson ts "$ts" \
        '{issue: $issue, failures: $failures, updated_at: $ts}' \
        > "$tmp_issue"
    mv "$tmp_issue" "$issue_file"

    say "FAILURES:${current_failures}"
    say "CAP:${FAILURE_CAP}"

    if [ "$current_failures" -ge "$FAILURE_CAP" ]; then
        # Label issue autospec:needs-human.
        "$GH" issue edit "$issue" --add-label "autospec:needs-human" \
            --repo "$repo" 2>/dev/null || true
        # Post a comment explaining the quarantine.
        "$GH" issue comment "$issue" \
            --repo "$repo" \
            --body "autospec-autonomous: issue quarantined after ${current_failures} consecutive failures (cap=${FAILURE_CAP}). Labeled \`autospec:needs-human\`. Manual review required before re-queuing." \
            2>/dev/null || true
        notify_op "autospec: quarantine" \
            "Issue #${issue} in ${repo} quarantined after ${current_failures} failures"
        say "DECISION:quarantine"
        exit 1
    fi
    say "DECISION:continue"
}

# ── Subcommand: main-health ───────────────────────────────────────────────────
# main-health --repo OWNER/REPO
cmd_main_health() {
    local repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "main-health: unknown option: $1" ;;
        esac
    done
    repo="$(resolve_repo "$repo")"

    # Use gh api (not ci-wait.sh which is per-PR).
    # endpoint: /repos/{owner}/{repo}/commits/main/status  → .state field
    local api_output
    api_output="$("$GH" api "repos/${repo}/commits/main/status" 2>/dev/null || echo "")"

    if [ -z "$api_output" ]; then
        # gh failed (network, auth, etc.) → treat as pending (conservative).
        say "DECISION:wait"
        say "REASON:gh-api-failed"
        return
    fi

    # Extract .state safely — never interpolate into jq test()
    local ci_state
    ci_state="$(printf '%s' "$api_output" | jq -r '.state // empty' 2>/dev/null || echo "")"

    case "$ci_state" in
        success)
            say "DECISION:continue"
            say "CI_STATE:success"
            ;;
        pending|"")
            say "DECISION:wait"
            say "CI_STATE:${ci_state:-unknown}"
            ;;
        failure|error)
            notify_op "autospec: main-health halt" \
                "Main branch CI is ${ci_state} for ${repo} — halting Tier-1 merges"
            say "DECISION:halt"
            say "CI_STATE:${ci_state}"
            exit 1
            ;;
        *)
            # Unknown state → conservative wait
            say "DECISION:wait"
            say "CI_STATE:${ci_state}"
            ;;
    esac
}

# ── Entry point ───────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
Usage: autonomous-resilience.sh <subcommand> [options]

Subcommands:
  state  write  --repo OWNER/REPO --status STATUS [--session SESSION]
  state  read   --repo OWNER/REPO
  lock   acquire --repo OWNER/REPO [--session SESSION]
  lock   release --repo OWNER/REPO [--session SESSION]
  lock   check   --repo OWNER/REPO
  quarantine  --repo OWNER/REPO --issue N [--failures N]
  main-health --repo OWNER/REPO

Outputs DECISION:<token> lines on stdout.
Exit 0 = ok; 1 = lock-held/quarantine/halt; 2 = usage error.
EOF
}

CMD="${1:-}"
if [ -z "$CMD" ]; then
    usage >&2
    exit 2
fi
shift

case "$CMD" in
    state)        cmd_state        "$@" ;;
    lock)         cmd_lock         "$@" ;;
    quarantine)   cmd_quarantine   "$@" ;;
    main-health)  cmd_main_health  "$@" ;;
    --help|-h)    usage; exit 0 ;;
    *)            die "unknown subcommand '${CMD}'. Run with --help for usage." ;;
esac
