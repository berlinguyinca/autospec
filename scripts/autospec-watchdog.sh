#!/usr/bin/env bash
# autospec-watchdog.sh — reclaim and nudge stalled autospec workers.
#
# The monitor should call this at startup, before candidate selection, and on
# its regular service-watch cadence to reconcile `process-heartbeats/*.json`
# files and detect stalled workers.
#
# Environment overrides:
#   AUTOSPEC_WATCHDOG_DIR              heartbeat directory (default: ~/.autospec/process-heartbeats);
#                                     AUTOSPEC_HEARTBEAT_DIR (writers' var) takes precedence so both agree
#   AUTOSPEC_WATCHDOG_REPO              override repo for gh calls (default: gh repo context)
#   AUTOSPEC_WATCHDOG_STALE_SECS         stale threshold (default: 1800)
#   AUTOSPEC_WATCHDOG_RECLAIM_SECS       reclaim threshold (default: 10800)
#   AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS claimed-step release threshold (default: 1800)
#   AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS nudge cooldown (default: 900)
#   AUTOSPEC_WATCHDOG_STATE_FILE         state file for nudge cooldown (default: ~/.autospec/watchdog-state.tsv)

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

if command -v autospec_runtime_config_get >/dev/null 2>&1; then
    _watchdog_base_cfg="$(autospec_runtime_config_get autonomous.heartbeat_dir "")"
    if [ -n "$_watchdog_base_cfg" ] && [ "$_watchdog_base_cfg" != "auto" ]; then
        WATCHDOG_BASE="$_watchdog_base_cfg"
    else
        WATCHDOG_BASE="$(autospec_runtime_config_path autonomous.watchdog.heartbeat_dir AUTOSPEC_WATCHDOG_DIR "${AUTOSPEC_HEARTBEAT_DIR:-$HOME/.autospec/process-heartbeats}")"
    fi
    WATCHDOG_REPO="$(autospec_runtime_config_get autonomous.repo "")"
    [ "$WATCHDOG_REPO" = "auto" ] && WATCHDOG_REPO=""
    [ -n "$WATCHDOG_REPO" ] || WATCHDOG_REPO="$(autospec_runtime_config_path autonomous.watchdog.repo AUTOSPEC_WATCHDOG_REPO "${AUTOSPEC_REPO:-}")"
    WATCHDOG_STALE_SECS="$(autospec_runtime_config_int autonomous.watchdog.stale_secs AUTOSPEC_WATCHDOG_STALE_SECS 1800)"
    WATCHDOG_RECLAIM_SECS="$(autospec_runtime_config_int autonomous.watchdog.reclaim_secs AUTOSPEC_WATCHDOG_RECLAIM_SECS 10800)"
    WATCHDOG_CLAIMED_TIMEOUT_SECS="$(autospec_runtime_config_int autonomous.watchdog.claimed_timeout_secs AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS 1800)"
    WATCHDOG_NUDGE_COOLDOWN_SECS="$(autospec_runtime_config_int autonomous.watchdog.nudge_cooldown_secs AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS 900)"
    STATE_FILE="$(autospec_runtime_config_path autonomous.watchdog.state_file AUTOSPEC_WATCHDOG_STATE_FILE "$HOME/.autospec/watchdog-state.tsv")"
else
    WATCHDOG_BASE="${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"
    WATCHDOG_REPO="${AUTOSPEC_WATCHDOG_REPO:-${AUTOSPEC_REPO:-}}"
    WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
    WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
    WATCHDOG_CLAIMED_TIMEOUT_SECS="${AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS:-1800}"
    WATCHDOG_NUDGE_COOLDOWN_SECS="${AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS:-900}"
    STATE_FILE="${AUTOSPEC_WATCHDOG_STATE_FILE:-$HOME/.autospec/watchdog-state.tsv}"
fi

# Orphaned-worktree GC (crash-resume design, Child 3). The GC pass scans this
# root for `wt-*` worktree directories and prunes only those that are provably
# safe to remove (no un-pushed commits, issue closed/unlabeled, no live
# heartbeat). Default root is /tmp where autospec runners create `/tmp/wt-*`.
if command -v autospec_runtime_config_path >/dev/null 2>&1; then
    WATCHDOG_GC_DIR="$(autospec_runtime_config_path autonomous.watchdog.gc_dir AUTOSPEC_WATCHDOG_GC_DIR /tmp)"
else
    WATCHDOG_GC_DIR="${AUTOSPEC_WATCHDOG_GC_DIR:-/tmp}"
fi
# Heartbeat is considered "live" if its ts is within this many seconds of now.
if command -v autospec_runtime_config_int >/dev/null 2>&1; then
    WATCHDOG_GC_HEARTBEAT_FRESH_SECS="$(autospec_runtime_config_int autonomous.watchdog.gc_heartbeat_fresh_secs AUTOSPEC_WATCHDOG_GC_HEARTBEAT_FRESH_SECS "$WATCHDOG_STALE_SECS")"
else
    WATCHDOG_GC_HEARTBEAT_FRESH_SECS="${AUTOSPEC_WATCHDOG_GC_HEARTBEAT_FRESH_SECS:-${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}}"
fi

WATCHDOG_LOG_PREFIX="[autospec-watchdog]"

# Sibling helpers (F2 liveness + F4 canonical slug). Resolved relative to this
# script so a checkout/worktree picks up its own copies; overridable for tests.
WATCHDOG_SCRIPT_DIR="$SCRIPT_DIR"
WORKER_LIVENESS_SH="${AUTOSPEC_WORKER_LIVENESS_SH:-$WATCHDOG_SCRIPT_DIR/worker-liveness.sh}"
REPO_SLUG_SH="${AUTOSPEC_REPO_SLUG_SH:-$WATCHDOG_SCRIPT_DIR/repo-slug.sh}"

if ! command -v gh >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX ERROR: gh CLI not found" >&2
    exit 1
fi

# ── Derive repo slug and scoped heartbeat dir ─────────────────────────────────

_resolve_repo_full() {
    local repo="${WATCHDOG_REPO:-}"
    if [ -z "$repo" ]; then
        repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    fi
    printf '%s' "$repo"
}

REPO_FULL="$(_resolve_repo_full)"
if [ -n "$REPO_FULL" ]; then
    # Reuse repo-slug.sh canonical-slug resolver (F4): prefers the canonical
    # owner__name dir but transparently falls back to legacy owner_name /
    # owner-name dirs (the legacy single-underscore form) so in-flight
    # heartbeats from pre-migration writers are still found. The documented
    # one-release legacy read-fallback lives inside resolve_slug_dir.
    if [ -x "$REPO_SLUG_SH" ]; then
        WATCHDOG_DIR="$(bash "$REPO_SLUG_SH" --resolve-dir "$WATCHDOG_BASE" "$REPO_FULL" 2>/dev/null || true)"
    fi
    if [ -z "${WATCHDOG_DIR:-}" ]; then
        # Degraded path (helper missing/non-exec): stay canonical so this reader
        # never keys legacy against the canonical writers' degraded fallback.
        WATCHDOG_DIR="${WATCHDOG_BASE}/$(printf '%s' "$REPO_FULL" | sed 's#/#__#')"
    fi
else
    WATCHDOG_DIR="$WATCHDOG_BASE"
fi

if [ ! -d "$WATCHDOG_BASE" ]; then
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0 live_owner_no_heartbeat=0"
    exit 0
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX WARN: jq CLI not found; skipping heartbeat reconciliation" >&2
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0 live_owner_no_heartbeat=0"
    exit 0
fi

# ── Migration: flat-format heartbeats → repo-scoped subdirs ──────────────────
# On each watchdog tick, scan WATCHDOG_BASE for flat-format *.json files (i.e.,
# files directly under WATCHDOG_BASE, not under a subdir).  Files with a `repo`
# field are moved to the correct <repo-slug>/ subdir; files older than 1 hour
# with no `repo` field are deleted.

FLAT_MIGRATION_STALE_SECS=3600
_migrate_flat_heartbeats() {
    local base="$1"
    local now="$2"
    for flat_hb in "$base"/*.json; do
        [ -f "$flat_hb" ] || continue
        local flat_issue
        flat_issue="$(basename "$flat_hb" .json)"
        if [[ ! "$flat_issue" =~ ^[0-9]+$ ]]; then
            continue
        fi
        local flat_repo
        flat_repo="$(jq -r '.repo // empty' "$flat_hb" 2>/dev/null || true)"
        if [ -n "$flat_repo" ]; then
            # Move to correct subdir. This CREATES/writes a slug dir, so it is a
            # WRITER and MUST emit the canonical owner__name form (F4) — same as
            # heartbeat-write.sh — so the resolve_slug_dir read path converges on
            # one canonical dir instead of splitting flat-migrated heartbeats
            # into a legacy dir.
            local dest_dir
            if [ -x "$REPO_SLUG_SH" ]; then
                dest_dir="${base}/$(bash "$REPO_SLUG_SH" --canonical "$flat_repo" 2>/dev/null || true)"
            fi
            if [ -z "${dest_dir:-}" ] || [ "$dest_dir" = "${base}/" ]; then
                # Degraded fallback stays canonical (owner__name), never legacy.
                dest_dir="${base}/$(printf '%s' "$flat_repo" | sed 's#/#__#')"
            fi
            mkdir -p "$dest_dir"
            mv "$flat_hb" "${dest_dir}/${flat_issue}.json"
            echo "$WATCHDOG_LOG_PREFIX migrated flat heartbeat #${flat_issue} → $(basename "$dest_dir")/" >&2
        else
            # No repo field: delete if older than 1 hour
            local flat_ts
            flat_ts="$(jq -r '.ts // empty' "$flat_hb" 2>/dev/null || true)"
            if [ -n "$flat_ts" ] && [[ "$flat_ts" =~ ^[0-9]+$ ]]; then
                local age=$(( now - flat_ts ))
                if [ "$age" -ge "$FLAT_MIGRATION_STALE_SECS" ]; then
                    rm -f "$flat_hb"
                    echo "$WATCHDOG_LOG_PREFIX deleted stale flat heartbeat #${flat_issue} (age=${age}s)" >&2
                fi
            else
                # Unparseable ts — delete
                rm -f "$flat_hb"
            fi
        fi
    done
}

now_ts="$(date -u +%s)"

_migrate_flat_heartbeats "$WATCHDOG_BASE" "$now_ts"

# NOTE: do NOT create WATCHDOG_DIR here. The read loop and GC tolerate a missing
# dir (every access is `[ -f ]`-guarded). repo-slug.sh --resolve-dir only returns
# the canonical owner__name path when no dir exists yet; eagerly creating it
# would shadow the legacy owner_name dir that heartbeat-write.sh creates later,
# orphaning real heartbeats. The dir is created by the writer, not the watchdog.

nudged=0
reclaimed=0
claimed_released=0
garbage_collected=0
invalid_schema=0
skipped=0
live_owner_no_heartbeat=0

if [ -n "$WATCHDOG_REPO" ]; then
    REPO_ARGS="--repo $WATCHDOG_REPO"
else
    REPO_ARGS=""
fi

STATE_LINES=""

load_state() {
    if [ -f "$STATE_FILE" ]; then
        STATE_LINES="$(awk '$1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { print $1 "\t" $2 }' "$STATE_FILE")"
    fi
}

state_get() {
    printf '%s\n' "$STATE_LINES" | awk -v issue="$1" '$1 == issue { print $2; exit }'
}

state_set() {
    issue="$1"
    ts="$2"
    STATE_LINES="$(printf '%s\n' "$STATE_LINES" | awk -v issue="$issue" 'NF && $1 != issue { print }')"
    if [ -n "$STATE_LINES" ]; then
        STATE_LINES="${STATE_LINES}
${issue}	${ts}"
    else
        STATE_LINES="${issue}	${ts}"
    fi
}

state_unset() {
    issue="$1"
    STATE_LINES="$(printf '%s\n' "$STATE_LINES" | awk -v issue="$issue" 'NF && $1 != issue { print }')"
}

issue_meta() {
    # shellcheck disable=SC2086
    gh issue view "$1" $REPO_ARGS \
        --json state,labels \
        --jq '.state + " " + ([.labels[].name] | join(","))' \
        2>/dev/null || true
}

run_state_comment_ids_for_issue() {
    issue="$1"
    [ -n "${REPO_FULL:-}" ] || return 1
    # Fetch via the REST comments endpoint because `gh issue view --json
    # comments` exposes GraphQL node ids, while DELETE needs the numeric REST
    # comment id.
    gh api --paginate "repos/$REPO_FULL/issues/$issue/comments?per_page=100" \
        --jq '[.[]? | select((.body // "") | contains("<!-- autospec-run-state:begin -->") and contains("<!-- autospec-run-state:end -->"))] | sort_by(.created_at, .id) | .[].id' \
        2>/dev/null
}

clear_run_state_comments_for_issue() {
    issue="$1"
    ids="$(run_state_comment_ids_for_issue "$issue")" || return 1
    for comment_id in $ids; do
        # shellcheck disable=SC2086
        gh api "repos/$REPO_FULL/issues/comments/$comment_id" -X DELETE >/dev/null 2>&1 || return 1
    done
    return 0
}

reclaim_issue() {
    local issue="$1"
    local age="$2"

    # Clear the stale authoritative run-state lease before making the issue
    # claimable again. Otherwise the next queue/claim cycle can briefly observe
    # `in-progress-by-bot` with the previous worker_id and report a fresh claim
    # for a dead worker. If GitHub cannot clear the lease, fail closed and leave
    # labels untouched for the next watchdog tick.
    clear_run_state_comments_for_issue "$issue" || return 1

    # shellcheck disable=SC2086
    gh issue edit "$issue" $REPO_ARGS \
        --remove-label in-progress-by-bot \
        --add-label auto-implement >/dev/null 2>&1 || return 1
    # shellcheck disable=SC2086
    gh issue comment "$issue" $REPO_ARGS \
        --body "autospec watchdog released and reclaimed this issue after ${age}s with no live owner." >/dev/null 2>&1 || true
}

nudge_issue() {
    local issue="$1"

    # shellcheck disable=SC2086
    gh issue comment "$issue" $REPO_ARGS \
        --body "autospec watchdog: please check in; if stuck, post blocker and clear in-progress-by-bot." \
        >/dev/null 2>&1 || return 1
}

save_state() {
    mkdir -p "$HOME/.autospec"
    if [ -z "$STATE_LINES" ]; then
        rm -f "$STATE_FILE"
        return
    fi
    tmp="$(mktemp "$HOME/.autospec/.watchdog-state.XXXXXX")"
    printf '%s\n' "$STATE_LINES" > "$tmp"
    mv "$tmp" "$STATE_FILE"
}

json_value() {
    key="$1"
    file="$2"
    jq -r --arg key "$key" '.[$key] // empty' "$file" 2>/dev/null || true
}

iso_to_epoch() {
    ts="$1"
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

heartbeat_schema_valid() {
    file="$1"
    issue="$2"

    hb_issue="$(json_value issue "$file")"
    hb_step="$(json_value step "$file")"
    hb_ts="$(json_value ts "$file")"

    case "$hb_issue" in
        "$issue") ;;
        *) return 1 ;;
    esac
    case "$hb_ts" in
        ''|*[!0-9]*) return 1 ;;
    esac
    case "$hb_step" in
        claimed|expand_start|worktree_ready|tests_started|tests_passed|pr_created|smoke_retry|reviewed|merged|failed) ;;
        *) return 1 ;;
    esac
    return 0
}

normalize_heartbeat() {
    file="$1"
    issue="$2"

    branch="$(json_value branch "$file")"
    step="$(json_value step "$file")"
    ts="$(json_value ts "$file")"
    pr="$(json_value pr "$file")"
    repo="$(json_value repo "$file")"
    tmp="${file}.tmp"
    jq -n \
        --arg issue "$issue" \
        --arg branch "$branch" \
        --arg step "$step" \
        --argjson ts "$ts" \
        --arg pr "$pr" \
        --arg repo "$repo" \
        '{issue:$issue,branch:$branch,step:$step,ts:$ts,pr:$pr,repo:$repo}' > "$tmp" \
        && mv "$tmp" "$file"
}

# ── Orphaned-worktree GC (crash-resume design, Child 3) ───────────────────────
#
# Prune a `wt-*` worktree via `git worktree remove --force` ONLY when ALL three
# data-integrity guards hold:
#   (1) NO un-pushed commits   — `git -C <wt> log --not --remotes` is empty
#   (2) issue closed/unlabeled — gh shows the issue not (OPEN AND still labeled
#                                auto-implement/in-progress-by-bot)
#   (3) NO live heartbeat      — no heartbeat for that issue has a fresh `ts`
#                                (within WATCHDOG_GC_HEARTBEAT_FRESH_SECS)
#
# This is a destructive, data-integrity-load-bearing pass: it NEVER recursively
# force-deletes a path and NEVER removes a worktree with un-pushed work or a
# live heartbeat. It
# only ever invokes `git worktree remove --force` after all three guards pass.
# The pass is bounded (it only globs WATCHDOG_GC_DIR/wt-*) and must not block or
# slow the reclaim loop; every guard short-circuits on the cheap local checks
# (rev-parse, log) before the single per-candidate `gh` call.

# True (0) if a heartbeat for $issue is live (fresh ts within the freshness
# window). Conservative: any parse failure is treated as "not live" so it does
# not by itself save a worktree — the un-pushed and issue-state guards remain.
_gc_heartbeat_is_live() {
    issue="$1"
    hb="${WATCHDOG_DIR}/${issue}.json"
    [ -f "$hb" ] || return 1
    hb_ts="$(json_value ts "$hb")"
    case "$hb_ts" in
        ''|*[!0-9]*) return 1 ;;
    esac
    hb_age=$(( now_ts - hb_ts ))
    [ "$hb_age" -lt 0 ] && hb_age=0
    [ "$hb_age" -lt "$WATCHDOG_GC_HEARTBEAT_FRESH_SECS" ]
}

gc_orphaned_worktrees() {
    command -v git >/dev/null 2>&1 || return 0
    for wt in "$WATCHDOG_GC_DIR"/wt-*; do
        [ -d "$wt" ] || continue
        # Must be a real git worktree before we touch it.
        git -C "$wt" rev-parse --is-inside-work-tree >/dev/null 2>&1 || continue

        branch="$(git -C "$wt" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        case "$branch" in
            ''|HEAD) continue ;;
        esac

        # Derive the issue number from the branch (e.g. feat/...-issue-700 or a
        # trailing numeric segment). Skip if we cannot identify an issue.
        issue="$(printf '%s' "$branch" | grep -oE '[0-9]+' | tail -1 || true)"
        case "$issue" in
            ''|*[!0-9]*) continue ;;
        esac

        # GUARD 1 — un-pushed commits. NEVER prune if any local commit is not on
        # a remote. This is the load-bearing data-integrity check.
        unpushed="$(git -C "$wt" log --not --remotes --oneline 2>/dev/null || echo "x")"
        [ -z "$unpushed" ] || continue

        # GUARD 3 — live heartbeat (cheap local file check; do it before gh).
        if _gc_heartbeat_is_live "$issue"; then
            continue
        fi

        # GUARD 2 — issue closed or unlabeled. Skip if the issue is still OPEN
        # AND still carries an in-flight label. issue_meta is "STATE labels".
        meta="$(issue_meta "$issue")"
        gc_state="${meta%% *}"
        gc_labels="${meta#* }"
        [ "$gc_state" = "$meta" ] && gc_labels=""
        if [ "$gc_state" = "OPEN" ]; then
            if printf '%s' ",${gc_labels}," | grep -qE ',(auto-implement|in-progress-by-bot),'; then
                continue
            fi
        fi

        # All three guards passed → safe to prune via git's own worktree removal.
        if git worktree remove --force "$wt" >/dev/null 2>&1; then
            garbage_collected=$((garbage_collected + 1))
            echo "$WATCHDOG_LOG_PREFIX gc: removed orphaned worktree $wt (issue #$issue, branch $branch)" >&2
        fi
    done
}

# GC-only mode: run just the orphaned-worktree GC pass and exit. Used by the
# watchdog GC bats fixture to exercise the real pass in isolation.
if [ "${AUTOSPEC_WATCHDOG_GC_ONLY:-}" = "1" ]; then
    gc_orphaned_worktrees
    printf 'service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=%s invalid_schema=0 skipped=0 live_owner_no_heartbeat=0\n' \
        "$garbage_collected"
    exit 0
fi

# ── GitHub-authority + liveness reclaim gate (F1+F2+F3) ───────────────────────
#
# The local heartbeat age is only a *trigger to check* — never the decision.
# Once a `claimed` heartbeat is older than the claimed-timeout, the GitHub
# `autospec-run-state` comment plus same-host PID-liveness are authoritative.

# Fetch the run-state comment body for an issue (the marked comment written by
# `autospec claim acquire` / `autospec claim release`). Defined before the main loop so the
# heartbeat path can consult it. Empty when absent (no marked comment).
# Exits non-zero on gh API failure so the caller can treat that as fail-safe.
run_state_body_for_issue() {
    issue="$1"
    # Pick the CAS-authoritative marked comment: autospec claim state keeps the
    # lowest-id (oldest) comment as the single owner and deletes losers, so in a
    # transient duplicate window we mirror that by sorting marked comments oldest
    # -first (createdAt, then id) and taking the first — never an arbitrary
    # array-order comment that could carry a loser's worker_id.
    # NOTE: do NOT suppress gh exit code with "|| true" here — the caller
    # (reclaim_decision) uses a non-zero exit as the fail-safe signal that
    # GitHub is unreachable; suppressing it would make a transient outage
    # indistinguishable from "no comment" and trigger a spurious reclaim.
    # shellcheck disable=SC2086
    gh issue view "$issue" $REPO_ARGS \
        --json comments \
        --jq '[.comments[]? | select((.body // "") | contains("<!-- autospec-run-state:begin -->") and contains("<!-- autospec-run-state:end -->"))] | sort_by(.createdAt, .id) | (.[0].body // "")' \
        2>/dev/null
}

# Strip the run-state JSON out of a marked comment body.
extract_run_state_json() {
    awk '
      /^<!-- autospec-run-state:begin -->$/ { inside=1; next }
      /^<!-- autospec-run-state:end -->$/ { inside=0; exit }
      inside { print }
    '
}

# worker_liveness <worker_id> → alive|dead|unknown (delegates to the F2 helper).
# Cross-host, malformed, or missing ids resolve to `unknown` so the caller falls
# back to GitHub-timestamp freshness.
worker_liveness() {
    wid="${1:-}"
    [ -n "$wid" ] || { printf 'unknown'; return 0; }
    if [ -x "$WORKER_LIVENESS_SH" ]; then
        bash "$WORKER_LIVENESS_SH" "$wid" 2>/dev/null || printf 'unknown'
    else
        printf 'unknown'
    fi
}

worktree_for_branch() {
    branch="$1"
    [ -n "$branch" ] || return 1
    command -v git >/dev/null 2>&1 || return 1
    git worktree list --porcelain 2>/dev/null | awk -v want="refs/heads/${branch}" '
      /^worktree / { wt=substr($0, 10); next }
      /^branch / && substr($0, 8) == want { print wt; exit }
    '
}

processes_in_worktree_with_proc() {
    wt="$1"
    proc_dir="${AUTOSPEC_WATCHDOG_PROC_DIR:-/proc}"
    [ "${AUTOSPEC_WATCHDOG_DISABLE_PROC:-0}" != "1" ] || { printf '0'; return 0; }
    [ -d "$proc_dir" ] || { printf '0'; return 0; }
    wt_real="$(cd "$wt" 2>/dev/null && pwd -P)" || { printf '0'; return 0; }
    count=0
    for cwd_link in "$proc_dir"/[0-9]*/cwd; do
        [ -e "$cwd_link" ] || continue
        pid="${cwd_link#"$proc_dir"/}"
        pid="${pid%/cwd}"
        [ "$pid" != "$$" ] || continue
        cwd="$(readlink "$cwd_link" 2>/dev/null || true)"
        case "$cwd" in
            "$wt_real"|"$wt_real"/*) count=$((count + 1)) ;;
        esac
    done
    printf '%s' "$count"
}

processes_in_worktree_with_lsof() {
    wt="$1"
    command -v lsof >/dev/null 2>&1 || { printf '0'; return 0; }
    wt_real="$(cd "$wt" 2>/dev/null && pwd -P)" || { printf '0'; return 0; }
    # Inspect cwd entries directly and filter paths ourselves. Avoid recursive
    # `lsof +D`, which walks the whole worktree and can be slow, and avoid broad
    # ps/grep command-line matching, which can self-match the watchdog process.
    lsof -w -Fn -d cwd 2>/dev/null | awk -v prefix="$wt_real" -v self="$$" '
      /^p/ { pid=substr($0, 2); next }
      /^n/ {
        path=substr($0, 2)
        if (pid != "" && pid != self && (path == prefix || index(path, prefix "/") == 1)) seen[pid]=1
      }
      END { for (pid in seen) count++; printf "%s", count + 0 }
    '
}

processes_in_worktree() {
    wt="$1"
    [ -n "$wt" ] && [ -d "$wt" ] || { printf '0'; return 0; }
    count="$(processes_in_worktree_with_proc "$wt")"
    case "$count" in *[!0-9]*|'') count=0 ;; esac
    if [ "$count" -gt 0 ]; then
        printf '%s' "$count"
        return 0
    fi
    processes_in_worktree_with_lsof "$wt"
}

note_live_owner_no_heartbeat() {
    issue="$1"
    branch="$2"
    count="$3"
    echo "$WATCHDOG_LOG_PREFIX live-owner-no-heartbeat issue #$issue branch=$branch active_local_processes=$count" >&2
}

active_local_worktree_process_count() {
    branch="$1"
    wt="$(worktree_for_branch "$branch")"
    [ -n "$wt" ] || { printf '0'; return 0; }
    processes_in_worktree "$wt"
}

pr_is_open() {
    pr="$1"
    [ -n "$pr" ] || return 1
    # shellcheck disable=SC2086
    state="$(gh pr view "$pr" $REPO_ARGS --json state --jq .state 2>/dev/null || true)"
    [ "$state" = "OPEN" ]
}

# Return the lowest-numbered open PR whose body links to the issue with a
# GitHub closing keyword. A linked open PR means the worker has handed ownership
# to the PR/checks path; the watchdog must not make the issue ready again while
# that PR is still open.
linked_open_pr_json_for_issue() {
    lop_issue="$1"
    # shellcheck disable=SC2086
    lop_prs="$(gh pr list $REPO_ARGS --state open --limit 100 --json number,state,body,statusCheckRollup 2>/dev/null)" || return 1
    printf '%s\n' "$lop_prs" | jq -c --arg issue "$lop_issue" '
      [ .[]
        | select(((.state // "OPEN") | ascii_upcase) == "OPEN")
        | select((.body // "") | test("(?i)(close[sd]?|fix(e[sd])?|resolve[sd]?)\\s+#" + $issue + "([^0-9]|$)"))
      ] | sort_by(.number) | .[0] // empty
    '
}
# reclaim_decision <issue> <window_secs> → "reclaim" | "hold"
#
# Decision table for a `claimed` heartbeat already past the claimed-timeout:
#   run-state absent / released / failed        → reclaim   (no live owner)
#   same-host worker, kill -0 alive             → hold      (#1055 regression)
#   same-host worker, kill -0 dead              → reclaim   (provably dead)
#   cross-host/unknown, GitHub ts fresh (<win)  → hold      (live sibling)
#   cross-host/unknown, GitHub ts stale (>=win) → reclaim
reclaim_decision() {
    rd_issue="$1"
    rd_window="$2"

    if ! rd_linked_pr="$(linked_open_pr_json_for_issue "$rd_issue")"; then
        printf 'hold'      # gh API unreachable — fail-safe: do not reclaim
        return 0
    fi
    if [ -n "$rd_linked_pr" ]; then
        printf 'hold'      # linked open PR owns the issue until PR finalization
        return 0
    fi

    # Fail-safe: if gh returns a non-zero exit (offline / rate-limited / auth
    # failure), we cannot prove the claim is stale → hold to never reclaim a
    # live claim we can't corroborate.  A truly absent run-state comment
    # produces exit 0 with an empty body (the --jq filter returns ""), which
    # is the only case we treat as "no authoritative owner → reclaim".
    if ! rd_body="$(run_state_body_for_issue "$rd_issue")"; then
        printf 'hold'      # gh API unreachable — fail-safe: do not reclaim
        return 0
    fi
    if [ -z "$rd_body" ]; then
        printf 'reclaim'   # run-state absent → no authoritative owner
        return 0
    fi

    rd_json="$(printf '%s\n' "$rd_body" | extract_run_state_json)"
    rd_state="$(printf '%s\n' "$rd_json" | jq -r '.state // .step // empty' 2>/dev/null || true)"
    rd_worker="$(printf '%s\n' "$rd_json" | jq -r '.worker_id // empty' 2>/dev/null || true)"
    rd_branch="$(printf '%s\n' "$rd_json" | jq -r '.branch // empty' 2>/dev/null || true)"
    rd_pr="$(printf '%s\n' "$rd_json" | jq -r '.pr // empty' 2>/dev/null || true)"
    rd_updated="$(printf '%s\n' "$rd_json" | jq -r '.updated_at // empty' 2>/dev/null || true)"

    case "$rd_state" in
        ''|released|failed)
            printf 'reclaim'   # absent/parse-fail or explicitly relinquished
            return 0
            ;;
    esac

    # F2 — same-host PID-liveness short-circuit.
    wl="$(worker_liveness "$rd_worker")"
    case "$wl" in
        alive)
            printf 'hold'      # provably-live same-host owner — never reclaim
            return 0
            ;;
    esac

    rd_local_processes="$(active_local_worktree_process_count "$rd_branch")"
    case "$rd_local_processes" in *[!0-9]*|'') rd_local_processes=0 ;; esac
    if [ "$rd_local_processes" -gt 0 ]; then
        if [ ! -f "${WATCHDOG_DIR}/${rd_issue}.json" ]; then
            note_live_owner_no_heartbeat "$rd_issue" "$rd_branch" "$rd_local_processes"
            printf 'hold_live_owner_no_heartbeat'
            return 0
        fi
        printf 'hold'          # active process in issue worktree — never reclaim
        return 0
    fi

    if [ "$wl" = "dead" ]; then
        printf 'reclaim'       # provably-dead same-host owner and no local worktree process
        return 0
    fi

    # F1 — cross-host / unknown: fall back to GitHub `updated_at` freshness.
    rd_epoch="$(iso_to_epoch "$rd_updated")"
    case "$rd_epoch" in *[!0-9]*|'') rd_epoch=0 ;; esac
    if [ "$rd_epoch" -le 0 ]; then
        printf 'reclaim'       # unparseable GitHub ts → treat as stale
        return 0
    fi
    rd_age=$(( now_ts - rd_epoch ))
    [ "$rd_age" -ge 0 ] || rd_age=0
    if [ "$rd_age" -ge "$rd_window" ]; then
        printf 'reclaim'       # GitHub claim is stale past the window
    else
        printf 'hold'          # GitHub claim is fresh — live sibling
    fi
    return 0
}

load_state

for hb in "$WATCHDOG_DIR"/*.json; do
    [ -f "$hb" ] || continue

    issue="${hb##*/}"
    issue="${issue%.json}"
    if [[ ! "$issue" =~ ^[0-9]+$ ]]; then
        skipped=$((skipped + 1))
        continue
    fi

    meta="$(issue_meta "$issue")"
    if [ -z "$meta" ]; then
        skipped=$((skipped + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    state="${meta%% *}"
    labels="${meta#* }"
    in_progress="false"
    if printf '%s' ",${labels}," | grep -q ",in-progress-by-bot,"; then
        in_progress="true"
    fi

    if [ "$state" != "OPEN" ] || [ "$in_progress" != "true" ]; then
        garbage_collected=$((garbage_collected + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    if ! heartbeat_schema_valid "$hb" "$issue"; then
        invalid_schema=$((invalid_schema + 1))
        rm -f "$hb"
        state_unset "$issue"
        continue
    fi

    normalize_heartbeat "$hb" "$issue"
    ts="$(json_value ts "$hb")"
    step="$(json_value step "$hb")"

    age=$(( now_ts - ts ))
    if [ "$age" -lt 0 ]; then
        age=0
    fi

    if [ "$step" = "claimed" ] && [ "$age" -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then
        # The stale heartbeat is only the trigger. GitHub authority + same-host
        # PID-liveness decide whether to actually reclaim (F1+F2+F3). A live
        # owner is never reclaimed — this closes the go-modules #1055 regression.
        decision="$(reclaim_decision "$issue" "$WATCHDOG_CLAIMED_TIMEOUT_SECS")"
        [ "$decision" = "hold_live_owner_no_heartbeat" ] && live_owner_no_heartbeat=$((live_owner_no_heartbeat + 1))
        if [ "$decision" = "reclaim" ]; then
            if reclaim_issue "$issue" "$age"; then
                claimed_released=$((claimed_released + 1))
                state_unset "$issue"
                rm -f "$hb"
            else
                skipped=$((skipped + 1))
            fi
        fi
        continue
    fi

    if [ "$age" -lt "$WATCHDOG_STALE_SECS" ]; then
        continue
    fi

    if [ "$age" -ge "$WATCHDOG_RECLAIM_SECS" ]; then
        # Gate the 3h TTL reclaim on the same GitHub-authority cross-check used
        # by the claimed-timeout path (F1+F2+F3 invariant, closes #1367).
        # A live worker is never reclaimed; gh API failure fail-safes to hold.
        decision="$(reclaim_decision "$issue" "$WATCHDOG_RECLAIM_SECS")"
        [ "$decision" = "hold_live_owner_no_heartbeat" ] && live_owner_no_heartbeat=$((live_owner_no_heartbeat + 1))
        if [ "$decision" = "reclaim" ]; then
            if reclaim_issue "$issue" "$age"; then
                reclaimed=$((reclaimed + 1))
                state_unset "$issue"
                rm -f "$hb"
            else
                skipped=$((skipped + 1))
            fi
        fi
        continue
    fi

    last_nudge="$(state_get "$issue")"
    last_nudge="${last_nudge:-0}"
    since_last_nudge=$((now_ts - last_nudge))
    if [ "$last_nudge" -eq 0 ] || [ "$since_last_nudge" -ge "$WATCHDOG_NUDGE_COOLDOWN_SECS" ]; then
        if nudge_issue "$issue"; then
            nudged=$((nudged + 1))
            state_set "$issue" "$now_ts"
        else
            skipped=$((skipped + 1))
        fi
    else
        skipped=$((skipped + 1))
    fi
done

reconcile_run_state_comments() {
    # shellcheck disable=SC2086
    issue_numbers="$(gh issue list $REPO_ARGS \
        --state open \
        --label in-progress-by-bot \
        --limit 200 \
        --json number \
        --jq '.[].number' 2>/dev/null || true)"
    for issue in $issue_numbers; do
        body="$(run_state_body_for_issue "$issue")"
        [ -n "$body" ] || continue
        run_state_json="$(printf '%s\n' "$body" | extract_run_state_json)"
        if ! printf '%s\n' "$run_state_json" | jq -e --argjson issue "$issue" \
            '.schema == 1 and .issue == $issue' >/dev/null 2>&1; then
            invalid_schema=$((invalid_schema + 1))
            continue
        fi

        step="$(printf '%s\n' "$run_state_json" | jq -r '.step // .state // empty')"
        updated_at="$(printf '%s\n' "$run_state_json" | jq -r '.updated_at // empty')"
        ttl="$(printf '%s\n' "$run_state_json" | jq -r '.ttl_seconds // empty')"
        pr="$(printf '%s\n' "$run_state_json" | jq -r '.pr // empty')"
        case "$ttl" in ''|*[!0-9]*) ttl="$WATCHDOG_RECLAIM_SECS" ;; esac
        [ -n "$updated_at" ] || continue

        updated_epoch="$(iso_to_epoch "$updated_at")"
        [ "$updated_epoch" -gt 0 ] || continue
        age=$((now_ts - updated_epoch))
        [ "$age" -ge 0 ] || age=0

        case "$step" in
            pr_created|awaiting_ci)
                if pr_is_open "$pr"; then
                    continue
                fi
                ;;
        esac

        if ! linked_pr_json="$(linked_open_pr_json_for_issue "$issue")"; then
            skipped=$((skipped + 1))
            continue
        fi
        if [ -n "$linked_pr_json" ]; then
            continue
        fi

        if [ "$step" = "claimed" ] && [ "$age" -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then
            # Missing-heartbeat reconciliation uses the same GitHub-authority +
            # same-host PID + local-worktree liveness gate as heartbeat-triggered
            # reclaim. A stale run-state comment is only the trigger, not proof
            # that the owner is dead.
            decision="$(reclaim_decision "$issue" "$WATCHDOG_CLAIMED_TIMEOUT_SECS")"
            [ "$decision" = "hold_live_owner_no_heartbeat" ] && live_owner_no_heartbeat=$((live_owner_no_heartbeat + 1))
            if [ "$decision" != "reclaim" ]; then
                continue
            fi
            if reclaim_issue "$issue" "$age"; then
                claimed_released=$((claimed_released + 1))
            else
                skipped=$((skipped + 1))
            fi
            continue
        fi

        if [ "$age" -ge "$ttl" ]; then
            decision="$(reclaim_decision "$issue" "$ttl")"
            [ "$decision" = "hold_live_owner_no_heartbeat" ] && live_owner_no_heartbeat=$((live_owner_no_heartbeat + 1))
            if [ "$decision" != "reclaim" ]; then
                continue
            fi
            if reclaim_issue "$issue" "$age"; then
                reclaimed=$((reclaimed + 1))
            else
                skipped=$((skipped + 1))
            fi
        fi
    done
}

reconcile_run_state_comments

# Orphaned-worktree GC: once per watchdog cycle, after reclaim, before summary.
gc_orphaned_worktrees

save_state
printf 'service-watch: nudged=%s reclaimed=%s claimed_released=%s garbage_collected=%s invalid_schema=%s skipped=%s live_owner_no_heartbeat=%s\n' \
    "$nudged" "$reclaimed" "$claimed_released" "$garbage_collected" "$invalid_schema" "$skipped" "$live_owner_no_heartbeat"
