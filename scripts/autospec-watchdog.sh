#!/usr/bin/env bash
# autospec-watchdog.sh — reclaim and nudge stalled autospec workers.
#
# The monitor should call this at startup, before candidate selection, and on
# its regular service-watch cadence to reconcile `process-heartbeats/*.json`
# files and detect stalled workers.
#
# Environment overrides:
#   AUTOSPEC_WATCHDOG_DIR              heartbeat directory (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_WATCHDOG_REPO              override repo for gh calls (default: gh repo context)
#   AUTOSPEC_WATCHDOG_STALE_SECS         stale threshold (default: 1800)
#   AUTOSPEC_WATCHDOG_RECLAIM_SECS       reclaim threshold (default: 10800)
#   AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS claimed-step release threshold (default: 300)
#   AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS nudge cooldown (default: 900)
#   AUTOSPEC_WATCHDOG_STATE_FILE         state file for nudge cooldown (default: ~/.autospec/watchdog-state.tsv)

set -eu

WATCHDOG_BASE="${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}"
WATCHDOG_REPO="${AUTOSPEC_WATCHDOG_REPO:-${AUTOSPEC_REPO:-}}"
WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
WATCHDOG_CLAIMED_TIMEOUT_SECS="${AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS:-300}"
WATCHDOG_NUDGE_COOLDOWN_SECS="${AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS:-900}"
STATE_FILE="${AUTOSPEC_WATCHDOG_STATE_FILE:-$HOME/.autospec/watchdog-state.tsv}"

# Orphaned-worktree GC (crash-resume design, Child 3). The GC pass scans this
# root for `wt-*` worktree directories and prunes only those that are provably
# safe to remove (no un-pushed commits, issue closed/unlabeled, no live
# heartbeat). Default root is /tmp where autospec runners create `/tmp/wt-*`.
WATCHDOG_GC_DIR="${AUTOSPEC_WATCHDOG_GC_DIR:-/tmp}"
# Heartbeat is considered "live" if its ts is within this many seconds of now.
WATCHDOG_GC_HEARTBEAT_FRESH_SECS="${AUTOSPEC_WATCHDOG_GC_HEARTBEAT_FRESH_SECS:-${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}}"

WATCHDOG_LOG_PREFIX="[autospec-watchdog]"

if ! command -v gh >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX ERROR: gh CLI not found" >&2
    exit 1
fi

# ── Derive repo slug and scoped heartbeat dir ─────────────────────────────────

_resolve_repo_slug() {
    local repo="${WATCHDOG_REPO:-}"
    if [ -z "$repo" ]; then
        repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    fi
    if [ -z "$repo" ]; then
        printf '' # fallback: empty slug → use base dir directly
        return
    fi
    printf '%s' "$repo" | tr '/' '_'
}

REPO_SLUG="$(_resolve_repo_slug)"
if [ -n "$REPO_SLUG" ]; then
    WATCHDOG_DIR="${WATCHDOG_BASE}/${REPO_SLUG}"
else
    WATCHDOG_DIR="$WATCHDOG_BASE"
fi

if [ ! -d "$WATCHDOG_BASE" ]; then
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0"
    exit 0
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "$WATCHDOG_LOG_PREFIX WARN: jq CLI not found; skipping heartbeat reconciliation" >&2
    printf '%s\n' "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0"
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
            # Move to correct subdir
            local dest_slug
            dest_slug="$(printf '%s' "$flat_repo" | tr '/' '_')"
            local dest_dir="${base}/${dest_slug}"
            mkdir -p "$dest_dir"
            mv "$flat_hb" "${dest_dir}/${flat_issue}.json"
            echo "$WATCHDOG_LOG_PREFIX migrated flat heartbeat #${flat_issue} → ${dest_slug}/" >&2
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

# Create scoped dir if needed
mkdir -p "$WATCHDOG_DIR"

nudged=0
reclaimed=0
claimed_released=0
garbage_collected=0
invalid_schema=0
skipped=0

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

reclaim_issue() {
    local issue="$1"
    local age="$2"

    # shellcheck disable=SC2086
    gh issue edit "$issue" $REPO_ARGS \
        --remove-label in-progress-by-bot \
        --add-label auto-implement >/dev/null 2>&1 || true
    # shellcheck disable=SC2086
    gh issue comment "$issue" $REPO_ARGS \
        --body "autospec watchdog reclaimed this issue after ${age}s of no check-in." >/dev/null 2>&1 || true
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
        claimed|worktree_ready|tests_started|tests_passed|pr_created|smoke_retry|reviewed|merged|failed) ;;
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
    printf 'service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=%s invalid_schema=0 skipped=0\n' \
        "$garbage_collected"
    exit 0
fi

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
        reclaim_issue "$issue" "$age"
        claimed_released=$((claimed_released + 1))
        state_unset "$issue"
        rm -f "$hb"
        continue
    fi

    if [ "$age" -lt "$WATCHDOG_STALE_SECS" ]; then
        continue
    fi

    if [ "$age" -ge "$WATCHDOG_RECLAIM_SECS" ]; then
        reclaim_issue "$issue" "$age"
        reclaimed=$((reclaimed + 1))
        state_unset "$issue"
        rm -f "$hb"
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

run_state_body_for_issue() {
    issue="$1"
    # shellcheck disable=SC2086
    gh issue view "$issue" $REPO_ARGS \
        --json comments \
        --jq '[.comments[]? | select((.body // "") | contains("<!-- autospec-run-state:begin -->") and contains("<!-- autospec-run-state:end -->")) | .body][0] // ""' \
        2>/dev/null || true
}

extract_run_state_json() {
    awk '
      /^<!-- autospec-run-state:begin -->$/ { inside=1; next }
      /^<!-- autospec-run-state:end -->$/ { inside=0; exit }
      inside { print }
    '
}

pr_is_open() {
    pr="$1"
    [ -n "$pr" ] || return 1
    # shellcheck disable=SC2086
    state="$(gh pr view "$pr" $REPO_ARGS --json state --jq .state 2>/dev/null || true)"
    [ "$state" = "OPEN" ]
}

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

        if [ "$step" = "claimed" ] && [ "$age" -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then
            reclaim_issue "$issue" "$age"
            claimed_released=$((claimed_released + 1))
            continue
        fi

        if [ "$age" -ge "$ttl" ]; then
            reclaim_issue "$issue" "$age"
            reclaimed=$((reclaimed + 1))
        fi
    done
}

reconcile_run_state_comments

# Orphaned-worktree GC: once per watchdog cycle, after reclaim, before summary.
gc_orphaned_worktrees

save_state
printf 'service-watch: nudged=%s reclaimed=%s claimed_released=%s garbage_collected=%s invalid_schema=%s skipped=%s\n' \
    "$nudged" "$reclaimed" "$claimed_released" "$garbage_collected" "$invalid_schema" "$skipped"
