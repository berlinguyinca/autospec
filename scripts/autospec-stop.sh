#!/usr/bin/env bash
# scripts/autospec-stop.sh — sentinel flag manager for graceful/immediate stop.
#
# Usage:
#   scripts/autospec-stop.sh [--graceful|--immediate|--status|--resume|--help]
#   scripts/autospec-stop.sh --abort-current-issue <ISSUE_N> <BRANCH> <STEP>
#
# Default: --graceful.
# Exit 0 on success, non-zero on argument error or gh failure.
# All gh calls scope to the cwd's git remote (gh repo view).
#
# Sentinel file: ~/.autospec/stop.flag
#   Line 1: graceful | immediate
#   Line 2: ISO-8601 UTC timestamp + <user>@<host>
#
# Dependencies: gh, git, date, awk, sed (no jq required)

set -eu

FLAG_FILE="${AUTOSPEC_STOP_FLAG_FILE:-${HOME}/.autospec/stop.flag}"
FLAG_DIR="$(dirname "$FLAG_FILE")"

# ---------- helpers ----------------------------------------------------------

info() { printf '[autospec-stop] %s\n' "$*"; }
warn() { printf '[autospec-stop] WARN: %s\n' "$*" >&2; }
err()  { printf '[autospec-stop] ERROR: %s\n' "$*" >&2; }

usage() {
    cat <<EOF
Usage: $0 [--graceful|--immediate|--status|--resume|--help]

Manage the autospec stop sentinel flag (${AUTOSPEC_STOP_FLAG_FILE:-~/.autospec/stop.flag}).

Flags:
  --graceful   Write sentinel mode=graceful; monitor exits after current issue.
  --immediate  Write sentinel mode=immediate; process(ISSUE) commits+pushes+marks paused at next boundary.
  --status     Report current sentinel state + paused-by-user issue count.
  --resume     Detect the monitor exit mode (silent-exit | prompt-overflow |
               clean | unknown) and print the matching recovery guidance, then
               strip paused-by-user labels, restore auto-implement, delete flag.
  --help       Show this help and exit.

Default (no flag): --graceful
EOF
}

# iso8601_now — emit UTC timestamp in ISO-8601 format (portable: macOS + Linux).
iso8601_now() {
    date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u +"%Y-%m-%dT%H:%M:%SZ"
}

# age_seconds TIMESTAMP — compute seconds since the given ISO-8601 stamp.
# Returns 0 on parse failure (conservative: don't suppress on bad timestamp).
age_seconds() {
    local ts="$1"
    local now epoch_now epoch_then
    now="$(date -u +%s 2>/dev/null || echo 0)"
    # macOS date -j vs GNU date -d
    epoch_then="$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0)"
    echo $(( now - epoch_then ))
}

# write_flag MODE — atomic write via temp+mv.
write_flag() {
    local mode="$1"
    local stamp user host line2
    stamp="$(iso8601_now)"
    user="$(id -un 2>/dev/null || echo unknown)"
    host="$(hostname -s 2>/dev/null || echo localhost)"
    line2="${stamp} ${user}@${host}"
    mkdir -p "$FLAG_DIR"
    local tmp
    tmp="$(mktemp "${FLAG_DIR}/.stop.flag.XXXXXX")"
    printf '%s\n%s\n' "$mode" "$line2" > "$tmp"
    mv "$tmp" "$FLAG_FILE"
}

# read_flag — sets FLAG_MODE, FLAG_LINE2 from flag file, or returns 1 if absent.
read_flag() {
    [ -f "$FLAG_FILE" ] || return 1
    FLAG_MODE="$(head -1 "$FLAG_FILE" 2>/dev/null || echo "")"
    FLAG_LINE2="$(sed -n '2p' "$FLAG_FILE" 2>/dev/null || echo "")"
}

# remaining_count — count open auto-implement issues via gh.
remaining_count() {
    gh issue list --label auto-implement --state open --json number --jq 'length' 2>/dev/null || echo "?"
}

# paused_count — count open paused-by-user issues.
paused_count() {
    gh issue list --label paused-by-user --state open --json number --jq 'length' 2>/dev/null || echo "?"
}

# ---------- commands ---------------------------------------------------------

cmd_graceful() {
    write_flag graceful
    local remaining
    remaining="$(remaining_count)"
    info "sentinel written: ${FLAG_FILE} = graceful"
    info "monitor will exit after current issue. ${remaining} issues remaining."
}

cmd_immediate() {
    write_flag immediate
    info "sentinel written: ${FLAG_FILE} = immediate"
    info "current process(ISSUE) will commit + push + mark paused at next step boundary."
}

cmd_status() {
    if ! read_flag 2>/dev/null; then
        info "no stop signal active"
        return 0
    fi

    local ts age_s age_human paused
    ts="$(printf '%s' "$FLAG_LINE2" | awk '{print $1}')"
    age_s="$(age_seconds "$ts")"

    if [ "$age_s" -gt 86400 ]; then
        age_human="${age_s}s (STALE — >24h)"
    elif [ "$age_s" -ge 3600 ]; then
        age_human="$(( age_s / 3600 ))h ago"
    elif [ "$age_s" -ge 60 ]; then
        age_human="$(( age_s / 60 ))m ago"
    else
        age_human="${age_s}s ago"
    fi

    local who
    who="$(printf '%s' "$FLAG_LINE2" | awk '{print $2}')"
    paused="$(paused_count)"

    printf 'flag: %s (set %s by %s)\n' "$FLAG_MODE" "$age_human" "$who"
    printf 'paused-by-user: %s issues\n' "$paused"
}

# detect_monitor_exit_mode — classify a halted Phase-4 monitor.
# Delegates to detect-monitor-exit-mode.sh (override via AUTOSPEC_DETECTOR for
# tests). Prints one of: silent-exit | prompt-overflow | clean | unknown.
# The exit-mode fingerprints are driven by the cross-tool memory entry
# docs/memory/feedback_monitor_silent_exit.md (the two known Phase-4 exit modes).
detect_monitor_exit_mode() {
    local detector="${AUTOSPEC_DETECTOR:-${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/detect-monitor-exit-mode.sh}"
    if [ -f "$detector" ]; then
        bash "$detector" 2>/dev/null || echo unknown
    else
        echo clean
    fi
}

# report_exit_mode_recovery MODE — print the standard recovery guidance for a
# detected exit mode (verbatim from feedback_monitor_silent_exit.md).
report_exit_mode_recovery() {
    local mode="$1"
    case "$mode" in
        clean)
            info "exit mode: clean — no halted monitor detected. Resume is a no-op."
            ;;
        silent-exit)
            info "exit mode: silent-exit — stale worktree + stuck in-progress-by-bot label, no PR."
            info "recovery: removing orphan worktree, restoring auto-implement; relaunch the monitor to pick it up fresh."
            ;;
        prompt-overflow)
            info "exit mode: prompt-overflow — monitor exceeded its context window (~185 tool calls)."
            info "recovery: restart with a fresh session/terser prompt; cap each monitor turn at fewer issues."
            ;;
        *)
            info "exit mode: unknown — stuck issue present but matches neither known fingerprint."
            info "falling back to plain label restore; inspect the issue manually before relaunching."
            ;;
    esac
}

cmd_resume() {
    # Memory-aware resume: first classify the monitor exit mode (driven by
    # docs/memory/feedback_monitor_silent_exit.md), print recovery guidance,
    # then perform the standard label restore + flag delete.
    local mode
    mode="$(detect_monitor_exit_mode)"
    report_exit_mode_recovery "$mode"

    # List paused issues; strip label + restore auto-implement; delete flag.
    local nums count=0

    nums="$(gh issue list --label paused-by-user --state open --json number --jq '.[].number' 2>/dev/null || echo "")"

    if [ -z "$nums" ]; then
        info "no paused-by-user issues; flag deleted."
    else
        local list=""
        for n in $nums; do
            gh issue edit "$n" --remove-label paused-by-user --add-label auto-implement 2>/dev/null || true
            count=$(( count + 1 ))
            list="${list}#${n}, "
        done
        list="${list%, }"
        info "resumed: ${count} issues (${list})"
    fi

    rm -f "$FLAG_FILE"
    info "flag deleted."
}

# cmd_abort_current_issue ISSUE_N BRANCH STEP
# Internal — called by process(ISSUE) sentinel check at step boundaries.
# Performs: WIP-commit (if dirty) → push → label-swap → insert Resume context block.
cmd_abort_current_issue() {
    local issue_n="$1"
    local branch="$2"
    local last_step="$3"

    info "abort-current-issue: issue #${issue_n} branch=${branch} last_step=${last_step}"

    # WIP commit if dirty
    if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
        git add -A
        git commit -m "chore: WIP — autospec stop (last step: ${last_step})" || true
    fi

    # Push branch
    git push -u origin "${branch}" 2>/dev/null || git push origin "${branch}" 2>/dev/null || true

    # Diff stat for resume context
    local diff_stat
    diff_stat="$(git diff --stat "origin/main" 2>/dev/null | tail -1 || echo "n/a")"

    # Build resume context block
    local resume_block
    resume_block="$(cat <<RESUME_BLOCK

## Resume context

- **Last step**: ${last_step} completed; aborted before next step
- **Branch**: ${branch} at $(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
- **Diff stat**: ${diff_stat}
- **Tests last seen**: see CI / last validate.sh run

<!-- autospec-resume:begin -->
*Auto-paused by user immediate stop on $(date -u +%Y-%m-%d).*
<!-- autospec-resume:end -->
RESUME_BLOCK
)"

    # Insert resume context block before first ## Dependencies (or at end-of-body).
    # Get current issue body.
    local current_body
    current_body="$(gh issue view "${issue_n}" --json body --jq .body 2>/dev/null || echo "")"

    local new_body
    if printf '%s' "$current_body" | grep -q '^## Dependencies'; then
        new_body="$(printf '%s' "$current_body" \
            | awk -v block="$resume_block" '/^## Dependencies/{print block; print; next} {print}')"
    else
        new_body="${current_body}${resume_block}"
    fi

    gh issue edit "${issue_n}" --body "$new_body" 2>/dev/null || true

    # Ensure paused-by-user label exists (idempotent, lavender colour)
    gh label create paused-by-user --color "d4c5f9" --force 2>/dev/null || true

    # Swap labels
    gh issue edit "${issue_n}" \
        --add-label paused-by-user \
        --remove-label in-progress-by-bot \
        2>/dev/null || true

    info "issue #${issue_n} marked paused-by-user; branch ${branch} pushed."
}

# ---------- main -------------------------------------------------------------

main() {
    local cmd="${1:---graceful}"

    case "$cmd" in
        --graceful)
            cmd_graceful
            ;;
        --immediate)
            cmd_immediate
            ;;
        --status)
            cmd_status
            ;;
        --resume)
            cmd_resume
            ;;
        --abort-current-issue)
            if [ $# -lt 4 ]; then
                err "--abort-current-issue requires <ISSUE_N> <BRANCH> <STEP>"
                exit 2
            fi
            cmd_abort_current_issue "$2" "$3" "$4"
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            err "unknown flag: $cmd"
            usage
            exit 2
            ;;
    esac
}

main "$@"
