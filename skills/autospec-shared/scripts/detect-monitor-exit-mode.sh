#!/usr/bin/env bash
# detect-monitor-exit-mode.sh — classify a halted Phase-4 autospec monitor.
#
# Inspects worktree state, GitHub label/PR state, and the latest heartbeat's
# tool-call count, then prints exactly one classification on stdout:
#
#   silent-exit     — stuck `in-progress-by-bot` issue, orphan worktree left
#                     behind, no PR filed (the canonical silent-exit fingerprint).
#   prompt-overflow — the monitor ran past the context window; the latest
#                     heartbeat's tool-call count exceeds the overflow threshold.
#   clean           — no halted monitor detected (no stuck issue / no orphan).
#   unknown         — a stuck issue exists but matches neither known fingerprint.
#
# This detector is driven by the cross-tool memory entry
#   docs/memory/feedback_monitor_silent_exit.md
# which documents the two known Phase-4 monitor exit modes (silent-exit via
# stale worktree + stuck label + no PR; prompt-overflow at ~185 tool calls).
# Keep this script and that memory entry in sync: the fingerprints below are
# the auditable encoding of that lesson.
#
# Usage:
#   detect-monitor-exit-mode.sh [--repo OWNER/NAME] [--help]
#
# Environment:
#   AUTOSPEC_WATCHDOG_DIR        heartbeat root (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_WORKTREE_BASE       worktree base to scan for orphans (default: /tmp)
#   AUTOSPEC_OVERFLOW_THRESHOLD  tool-call count that signals overflow (default: 180)
#
# Exit 0 on success (classification printed), non-zero on argument error.
# Dependencies: gh, jq, git, find, awk (gh/jq degrade gracefully if absent).

set -eu

REPO=""
HB_BASE="${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}"
WT_BASE="${AUTOSPEC_WORKTREE_BASE:-/tmp}"
# ~185 tool calls is where the empirical "Prompt is too long" overflow hit
# (see feedback_monitor_silent_exit.md, third confirmation). 180 leaves margin.
OVERFLOW_THRESHOLD="${AUTOSPEC_OVERFLOW_THRESHOLD:-180}"

usage() {
    cat <<EOF
Usage: $0 [--repo OWNER/NAME] [--help]

Classify a halted Phase-4 autospec monitor into one of four exit modes
(driven by docs/memory/feedback_monitor_silent_exit.md):

  silent-exit       stuck in-progress-by-bot issue + orphan worktree + no PR
  prompt-overflow   latest heartbeat tool-call count > threshold (~185)
  clean             no halted monitor detected
  unknown           stuck issue present but matches neither fingerprint

Environment:
  AUTOSPEC_WATCHDOG_DIR        heartbeat root (default: ~/.autospec/process-heartbeats)
  AUTOSPEC_WORKTREE_BASE       worktree base scanned for orphans (default: /tmp)
  AUTOSPEC_OVERFLOW_THRESHOLD  overflow tool-call threshold (default: 180)
EOF
}

# repo_slug OWNER/NAME -> OWNER_NAME (matches watchdog heartbeat subdir scheme).
repo_slug() {
    printf '%s\n' "$1" | tr '/' '_'
}

# latest_heartbeat DIR — print path of newest *.json heartbeat, or empty.
latest_heartbeat() {
    local dir="$1"
    [ -d "$dir" ] || return 0
    # Newest by mtime; portable across macOS/Linux via ls -t.
    ls -t "$dir"/*.json 2>/dev/null | head -1 || true
}

# json_field FILE KEY — extract a scalar JSON field (jq if present, awk fallback).
json_field() {
    local file="$1" key="$2"
    [ -f "$file" ] || return 0
    if command -v jq >/dev/null 2>&1; then
        jq -r --arg k "$key" '.[$k] // empty' "$file" 2>/dev/null || true
    else
        # Fallback: grab "key":VALUE or "key":"VALUE".
        awk -v k="\"$key\"" '
            { n=split($0, a, ","); for (i=1;i<=n;i++) {
                if (a[i] ~ k) {
                    sub(/^[^:]*:[[:space:]]*/, "", a[i]);
                    gsub(/["{} ]/, "", a[i]);
                    print a[i]; exit
                }
            } }' "$file"
    fi
}

# count_stuck_issues — number of open in-progress-by-bot issues (0 if gh absent).
count_stuck_issues() {
    command -v gh >/dev/null 2>&1 || { echo 0; return 0; }
    local repo_args="" raw
    [ -n "$REPO" ] && repo_args="--repo $REPO"
    # shellcheck disable=SC2086
    raw="$(gh issue list $repo_args --label in-progress-by-bot --state open \
        --json number --jq 'length' 2>/dev/null || echo 0)"
    sanitize_count "$raw"
}

# sanitize_count RAW — coerce gh/jq output to a non-negative integer.
# Accepts a bare integer (jq `length`) or a JSON array (count its elements).
sanitize_count() {
    local raw="$1"
    case "$raw" in
        ''|'[]') echo 0 ;;
        *[!0-9]*)
            # Not a bare integer (e.g. a JSON array): count "number" keys.
            printf '%s' "$raw" | grep -o '"number"' | wc -l | tr -d ' ' ;;
        *) echo "$raw" ;;
    esac
}

# pr_exists BRANCH — 1 if an open PR exists for the branch, else 0.
pr_exists() {
    local branch="$1"
    command -v gh >/dev/null 2>&1 || { echo 0; return 0; }
    [ -n "$branch" ] || { echo 0; return 0; }
    local repo_args=""
    [ -n "$REPO" ] && repo_args="--repo $REPO"
    local n
    # shellcheck disable=SC2086
    n="$(sanitize_count "$(gh pr list $repo_args --head "$branch" --state open \
        --json number --jq 'length' 2>/dev/null || echo 0)")"
    [ "$n" -gt 0 ] 2>/dev/null && echo 1 || echo 0
}

# orphan_worktree_exists BRANCH — 1 if a worktree dir for the branch lingers.
orphan_worktree_exists() {
    local branch="$1"
    # Branch feat/stop-resume-516 -> worktree dirs are named wt-feat-stop-resume-516
    # or wt-<slug>; scan WT_BASE for any dir whose name embeds the branch slug.
    local slug
    slug="$(printf '%s' "$branch" | tr '/' '-')"
    [ -n "$slug" ] || { echo 0; return 0; }
    if find "$WT_BASE" -maxdepth 1 -type d -name "*${slug}*" 2>/dev/null | grep -q .; then
        echo 1
    else
        echo 0
    fi
}

# ---------- main -------------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) shift; REPO="${1:-}" ;;
        --repo=*) REPO="${1#--repo=}" ;;
        --help|-h) usage; exit 0 ;;
        *) printf 'error: unknown flag: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

# Resolve repo from the cwd's git remote when not supplied.
if [ -z "$REPO" ] && command -v gh >/dev/null 2>&1; then
    REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || echo "")"
fi

slug="$(repo_slug "${REPO:-unknown_unknown}")"
HB_DIR="$HB_BASE/$slug"

stuck_count="$(count_stuck_issues)"
hb="$(latest_heartbeat "$HB_DIR")"

# No stuck issue => the monitor either never halted or recovered. Clean.
if [ "${stuck_count:-0}" -eq 0 ] 2>/dev/null; then
    echo clean
    exit 0
fi

# A stuck issue exists. Read the latest heartbeat for step + tool-call count.
step=""
tool_calls=0
branch=""
if [ -n "$hb" ]; then
    step="$(json_field "$hb" step)"
    branch="$(json_field "$hb" branch)"
    tc="$(json_field "$hb" tool_calls)"
    case "$tc" in (''|*[!0-9]*) tc=0 ;; esac
    tool_calls="$tc"
fi

# A terminal heartbeat step means the issue's label lag is expected post-merge
# (squash-merge preserves in-progress-by-bot as historical metadata — see the
# third confirmation in feedback_monitor_silent_exit.md). Treat as clean.
case "$step" in
    merged|done|complete|completed) echo clean; exit 0 ;;
esac

# prompt-overflow fingerprint: tool-call count past the overflow threshold.
if [ "${tool_calls:-0}" -ge "$OVERFLOW_THRESHOLD" ] 2>/dev/null; then
    echo prompt-overflow
    exit 0
fi

# silent-exit fingerprint: orphan worktree left behind + no PR for the branch.
orphan="$(orphan_worktree_exists "$branch")"
has_pr="$(pr_exists "$branch")"
if [ "$orphan" -eq 1 ] && [ "$has_pr" -eq 0 ]; then
    echo silent-exit
    exit 0
fi

# Stuck, but matches neither known fingerprint.
echo unknown
exit 0
