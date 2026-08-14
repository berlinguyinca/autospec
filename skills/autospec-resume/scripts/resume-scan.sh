#!/usr/bin/env bash
# resume-scan.sh — scan durable run-state + heartbeats on a FRESH start and
# decide whether to auto-continue an interrupted autospec run.
#
# Implements docs/specs/2026-06-03-crash-resume-design.md §Error handling:
#   - Auto-resume pre-conditions (ALL required, else exit 0, no relaunch):
#       (a) >=1 crashed in-progress-by-bot issue, or an open auto-implement
#           issue with an exact stale heartbeat-pending:none startup sentinel
#       (b) no ~/.autospec/stop.flag
#       (c) no issue labeled paused-by-user
#       (d) not all issues closed
#   - Crash-vs-live: ordinary in-progress work is eligible only when its age,
#     computed from run-state SERVER updated_at (never a local clock — mirrors
#     scripts/autospec-watchdog.sh:386-405), satisfies
#       step=claimed && age>=CLAIMED_TIMEOUT (300)  OR  age>=RECLAIM (10800).
#     An auto-implement issue is eligible only for the exact stale startup
#     sentinel; Rust owns its CAS recovery. Otherwise do NOT steal.
#   - Idempotency / NO second lock: this script NEVER writes a run-state comment
#     and adds NO new lock. The relaunched monitor claims through the EXISTING
#     GitHub CAS lock (`autospec claim state read`, lowest-comment-id wins). Two concurrent
#     resumes therefore converge to exactly one claim at the existing CAS.
#   - Cross-host: --resume-partial re-attaches /tmp/wt-<branch> only when the
#     heartbeat host == $(hostname). Cross-host or missing host MUST
#     clean-restart off origin/main.
#   - Attempt cap: capped at AUTOSPEC_RESUME_MAX_ATTEMPTS (default 3) consecutive
#     attempts without forward progress (an issue merged). Counter resets on a
#     merged issue.
#
# Usage:
#   resume-scan.sh [--repo <owner/name>] [--resume-partial] [--dry-run]
#
# Exit codes / output (mirrors the skill table):
#   0  nothing to resume / --dry-run    -> one-line reason on stdout
#   0  resumed                          -> "resuming <repo>: <N> issue(s); command=<...>"
#   1  hard error (bad args, no gh)     -> error to stderr
#
# Environment:
#   AUTOSPEC_STATE_DIR              base for stop.flag (default: ~/.autospec)
#   AUTOSPEC_BIN                   path to the Rust autospec command (default: autospec)
#   AUTOSPEC_HEARTBEAT_READ_SH     path to heartbeat-read.sh (auto-resolved)
#   AUTOSPEC_RUN_REGISTRY_SH       path to autospec-run-registry.sh (auto-resolved)
#   AUTOSPEC_RESUME_ATTEMPTS_SH    path to resume-attempts.sh (auto-resolved)
#   AUTOSPEC_HOST                  hostname override (tests)
#   WATCHDOG_CLAIMED_TIMEOUT_SECS  default 300
#   WATCHDOG_RECLAIM_SECS          default 10800

set -eu

STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

CLAIMED_TIMEOUT="${WATCHDOG_CLAIMED_TIMEOUT_SECS:-300}"
RECLAIM_SECS="${WATCHDOG_RECLAIM_SECS:-10800}"

err() { printf 'resume-scan: %s\n' "$1" >&2; }
die() { err "$1"; exit 1; }
say() { printf '%s\n' "$1"; }   # one-line decision/reason on stdout

# ── Resolve sibling helper scripts (checkout layout or installed layout). ──────
resolve_helper() {
    # $1 env override, $2..$N candidate paths
    override="$1"; shift
    if [ -n "$override" ] && [ -f "$override" ]; then printf '%s' "$override"; return; fi
    for c in "$@"; do
        [ -f "$c" ] && { printf '%s' "$c"; return; }
    done
    printf ''
}

AUTOSPEC_CLAIM_BIN="${AUTOSPEC_BIN:-}"
if [ -z "$AUTOSPEC_CLAIM_BIN" ] && [ -x "$SCRIPT_DIR/../../../target/debug/autospec" ]; then
    AUTOSPEC_CLAIM_BIN="$SCRIPT_DIR/../../../target/debug/autospec"
fi
[ -n "$AUTOSPEC_CLAIM_BIN" ] || AUTOSPEC_CLAIM_BIN="autospec"
HEARTBEAT_READ_SH="$(resolve_helper "${AUTOSPEC_HEARTBEAT_READ_SH:-}" \
    "$SCRIPT_DIR/../../autospec-run/scripts/heartbeat-read.sh" \
    "$STATE_DIR/scripts/heartbeat-read.sh")"
RUN_REGISTRY_SH="$(resolve_helper "${AUTOSPEC_RUN_REGISTRY_SH:-}" \
    "$SCRIPT_DIR/../../../scripts/autospec-run-registry.sh" \
    "$STATE_DIR/scripts/autospec-run-registry.sh")"
RESUME_ATTEMPTS_SH="$(resolve_helper "${AUTOSPEC_RESUME_ATTEMPTS_SH:-}" \
    "$SCRIPT_DIR/resume-attempts.sh" \
    "$STATE_DIR/scripts/resume-attempts.sh")"

# ── Arg parse ─────────────────────────────────────────────────────────────────
REPO=""
RESUME_PARTIAL=0
DRY_RUN=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo)           REPO="${2:-}"; shift 2 ;;
        --resume-partial) RESUME_PARTIAL=1; shift ;;
        --dry-run)        DRY_RUN=1; shift ;;
        --help|-h)
            sed -n '2,40p' "$0"; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

command -v gh >/dev/null 2>&1 || die "gh CLI not found"
command -v jq >/dev/null 2>&1 || die "jq not found"

if [ -z "$REPO" ]; then
    REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$REPO" ] || die "--repo is required when gh cannot infer it"

HOST="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"

now_epoch() { date -u +%s; }
iso_to_epoch() {
    ts="$1"
    [ -n "$ts" ] || { echo 0; return; }
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

# ── Pre-condition (b): stop.flag present -> exit 0, no relaunch. ───────────────
if [ -f "$STATE_DIR/stop.flag" ]; then
    say "stop.flag present"
    exit 0
fi

# ── Gather issues by label. We query GitHub once per label set. ───────────────
# in-progress-by-bot issues (candidates for resume):
in_progress="$(gh issue list --repo "$REPO" --label in-progress-by-bot \
    --state open --json number --jq '.[].number' 2>/dev/null || true)"
# paused-by-user issues (pre-condition c):
paused="$(gh issue list --repo "$REPO" --label paused-by-user \
    --state all --json number --jq '.[].number' 2>/dev/null || true)"
# open auto-implement OR in-progress issues (pre-condition d):
open_any="$(gh issue list --repo "$REPO" --state open \
    --label auto-implement --json number --jq '.[].number' 2>/dev/null || true)"
open_any="$(printf '%s\n%s\n' "$open_any" "$in_progress" | sed '/^$/d' | sort -u)"

# ── Pre-condition (c): a paused-by-user issue -> exit 0. ──────────────────────
if [ -n "$paused" ]; then
    say "paused-by-user present"
    exit 0
fi

# ── Pre-condition (d): not all issues closed. If neither auto-implement nor ───
# in-progress issues are open, there is nothing to resume.
if [ -z "$(printf '%s' "$open_any" | sed '/^$/d')" ]; then
    say "all issues closed"
    exit 0
fi

# ── Pre-condition (a) + crash-vs-live + cross-host: walk in-progress issues. ──
# An issue is eligible only when run-state is present, its heartbeat step is not
# terminal, AND server-updated_at age crosses the crash threshold.
eligible_count=0
eligible_issues=""
recovery_count=0
recovery_issues=""
partial_branch=""
saw_in_progress_nonterminal=0

scan_issues="$(printf '%s\n%s\n' "$in_progress" "$open_any" | sed '/^$/d' | sort -u)"
for issue in $scan_issues; do
    [ -n "$issue" ] || continue

    is_in_progress=0
    for active_issue in $in_progress; do
        if [ "$active_issue" = "$issue" ]; then
            is_in_progress=1
            break
        fi
    done

    # Heartbeat (machine-local): determine terminal step + host.
    hb=""
    if [ -n "$HEARTBEAT_READ_SH" ]; then
        hb="$(bash "$HEARTBEAT_READ_SH" --issue "$issue" --repo "$REPO" 2>/dev/null || true)"
    fi
    hb_step=""
    hb_host=""
    hb_branch=""
    if [ -n "$hb" ]; then
        hb_step="$(printf '%s' "$hb" | jq -r '.step // empty' 2>/dev/null || true)"
        # Missing host -> empty -> treated as unknown (never partial-eligible).
        hb_host="$(printf '%s' "$hb" | jq -r '.host // empty' 2>/dev/null || true)"
        hb_branch="$(printf '%s' "$hb" | jq -r '.branch // empty' 2>/dev/null || true)"
    fi
    case "$hb_step" in
        merged|failed) continue ;;   # terminal heartbeat -> not a crash to resume
    esac

    # Run-state (durable, cross-machine): must exist; gives SERVER updated_at.
    rs=""
    rs="$("$AUTOSPEC_CLAIM_BIN" claim state read --issue "$issue" --repo "$REPO" 2>/dev/null || true)"
    [ -n "$rs" ] || continue   # no durable run-state -> not a resumable claim
    printf '%s' "$rs" | jq -e 'type == "object"' >/dev/null 2>&1 || continue

    rs_step="$(printf '%s' "$rs" | jq -r '.step // .state // empty' 2>/dev/null || true)"
    rs_updated="$(printf '%s' "$rs" | jq -r '.updated_at // empty' 2>/dev/null || true)"
    rs_branch="$(printf '%s' "$rs" | jq -r '.branch // empty' 2>/dev/null || true)"
    [ -n "$rs_branch" ] || rs_branch="$hb_branch"

    # A claim can fail after the authoritative ref is created but before the
    # label changes from auto-implement to in-progress-by-bot. Detect only the
    # exact startup sentinel and exact issue/repository identity. Recovery is
    # deferred until the relaunch gates pass so a missing registry or attempt
    # cap cannot mutate queue state without starting the monitor.
    if [ "$rs_step" = "heartbeat-pending:none" ]; then
        printf '%s' "$rs" | jq -e \
            --arg repo "$REPO" --argjson issue "$issue" \
            'type == "object" and
             .state == "claimed" and
             .step == "heartbeat-pending:none" and
             .repo == $repo and
             .issue == $issue and
             (.updated_at | type == "string" and length > 0)' \
            >/dev/null 2>&1 || continue
        updated_epoch="$(iso_to_epoch "$rs_updated")"
        [ "$updated_epoch" -gt 0 ] || continue
        age=$(( $(now_epoch) - updated_epoch ))
        [ "$age" -ge 0 ] || age=0
        [ "$age" -ge "$CLAIMED_TIMEOUT" ] || continue
        recovery_count=$((recovery_count + 1))
        recovery_issues="$recovery_issues $issue"
        continue
    fi

    # Ordinary interrupted work remains label-gated as before. Auto-implement
    # issues enter this scan only for the exact stale-startup recovery above.
    [ "$is_in_progress" -eq 1 ] || continue
    saw_in_progress_nonterminal=1

    # Crash-vs-live off SERVER updated_at (never local clock).
    updated_epoch="$(iso_to_epoch "$rs_updated")"
    [ "$updated_epoch" -gt 0 ] || continue
    age=$(( $(now_epoch) - updated_epoch ))
    [ "$age" -ge 0 ] || age=0

    crashed=0
    if [ "$rs_step" = "claimed" ] && [ "$age" -ge "$CLAIMED_TIMEOUT" ]; then
        crashed=1
    elif [ "$age" -ge "$RECLAIM_SECS" ]; then
        crashed=1
    fi
    if [ "$crashed" -eq 0 ]; then
        # Fresh server heartbeat: assume a live/slow worker elsewhere; do NOT steal.
        continue
    fi

    eligible_count=$((eligible_count + 1))
    eligible_issues="$eligible_issues $issue"

    # Cross-host gate for --resume-partial: only re-attach /tmp/wt-<branch> when
    # the heartbeat host matches THIS host. Cross-host / missing host -> clean.
    if [ "$RESUME_PARTIAL" -eq 1 ] && [ -z "$partial_branch" ]; then
        if [ -n "$hb_host" ] && [ "$hb_host" = "$HOST" ]; then
            partial_branch="$rs_branch"
        fi
    fi
done

# ── Pre-condition (a): need >=1 eligible (non-terminal, crashed) issue. ───────
if [ "$eligible_count" -eq 0 ] && [ "$recovery_count" -eq 0 ]; then
    if [ "$saw_in_progress_nonterminal" -eq 1 ]; then
        say "no crashed in-progress run-state (live/slow worker not stolen)"
    else
        say "no open in-progress run-state"
    fi
    exit 0
fi

# ── Attempt cap (checked before incrementing / relaunching). ──────────────────
if [ -n "$RESUME_ATTEMPTS_SH" ]; then
    if bash "$RESUME_ATTEMPTS_SH" at-cap --repo "$REPO" 2>/dev/null; then
        cap="$(bash "$RESUME_ATTEMPTS_SH" cap 2>/dev/null || echo 3)"
        say "attempt cap reached (${cap})"
        exit 0
    fi
fi

# ── Resolve the durable relaunch command from the registry. ───────────────────
resume_command=""
if [ -n "$RUN_REGISTRY_SH" ]; then
    reg="$(bash "$RUN_REGISTRY_SH" read --repo "$REPO" 2>/dev/null || true)"
    [ -n "$reg" ] && resume_command="$(printf '%s' "$reg" | jq -r '.resume_command // empty' 2>/dev/null || true)"
fi
if [ -z "$resume_command" ]; then
    say "no durable resume_command in registry"
    exit 0
fi

restart_mode="clean-restart off origin/main"
if [ -n "$partial_branch" ]; then
    restart_mode="resume-partial /tmp/wt-${partial_branch} (host match)"
fi

if [ "$DRY_RUN" -eq 1 ]; then
    total_count=$((eligible_count + recovery_count))
    recovery_note=""
    if [ "$recovery_count" -gt 0 ]; then
        recovery_note="would recover stale startup [${recovery_issues# }]; "
    fi
    say "[dry-run] ${recovery_note}would resume $REPO: ${total_count} issue(s) [${eligible_issues# }${recovery_issues}]; mode=${restart_mode}; command=${resume_command}"
    exit 0
fi

# Recover exact stale-startup claims through the Rust CAS boundary. A concurrent
# owner, fresh/live evidence, or malformed/mismatched result leaves the claim
# untouched and excludes it from this relaunch.
for issue in $recovery_issues; do
    recovery="$("$AUTOSPEC_CLAIM_BIN" claim state recover-stale-startup \
        --issue "$issue" --repo "$REPO" --timeout-seconds "$CLAIMED_TIMEOUT" 2>/dev/null)" || continue
    printf '%s' "$recovery" | jq -e \
        --arg repo "$REPO" --argjson issue "$issue" \
        'type == "object" and .recovered == true and .repo == $repo and .issue == $issue' \
        >/dev/null || continue
    eligible_count=$((eligible_count + 1))
    eligible_issues="$eligible_issues $issue"
done

if [ "$eligible_count" -eq 0 ]; then
    say "no stale startup claim recovered"
    exit 0
fi

# Count this attempt (durable, capped). Resume itself writes NO run-state
# comment and adds NO lock — the relaunched monitor claims via the existing CAS.
if [ -n "$RESUME_ATTEMPTS_SH" ]; then
    bash "$RESUME_ATTEMPTS_SH" inc --repo "$REPO" >/dev/null 2>&1 || true
fi

say "resuming $REPO: ${eligible_count} issue(s); mode=${restart_mode}; command=${resume_command}"
# Execute the durably-captured relaunch command. It is the command /autospec-run
# itself captured for this repo (registry write), so it is not attacker-supplied
# from an arbitrary /tmp path.
# shellcheck disable=SC2086
eval "$resume_command"
