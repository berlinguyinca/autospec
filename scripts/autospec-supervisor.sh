#!/usr/bin/env bash
# autospec-supervisor.sh — external boot supervisor for crash-resume (child 2 of
# docs/specs/2026-06-03-crash-resume-design.md).
#
# Runs once at boot (launchd / systemd / @reboot cron — installed by
# autospec-supervisor-install.sh). For each entry in the durable active-runs
# registry (~/.autospec/active-runs/<repo-slug>.json), it:
#   1. reads the registry entry's repo,
#   2. INDEPENDENTLY confirms via GitHub that the repo has >=1 OPEN issue
#      labeled `in-progress-by-bot` (a confirmed open in-progress run-state),
#   3. only then invokes `/autospec-resume --repo <repo>` (resume-scan.sh),
#      which re-confirms crash-vs-live and resolves the durable resume_command.
#
# SECURITY (spec §Error handling "Supervisor safety"): the supervisor NEVER
# execs the registry's raw `resume_command` directly. It only ever delegates to
# /autospec-resume for a repo it has *independently confirmed* has an open
# in-progress run on GitHub. A poisoned/attacker-edited `resume_command` in the
# registry is therefore never executed by the supervisor — resume-scan.sh owns
# the relaunch, and only after its own GitHub crash-vs-live confirmation.
#
# BOOT-THRASH SAFETY: when NO registry entry has a confirmed open run, the
# supervisor prints one line, exits 0, and does NOT re-arm / reschedule itself.
# The boot unit fires once per boot; the supervisor never loops or re-arms.
#
# Usage:
#   autospec-supervisor.sh [--dry-run]
#
# Exit codes:
#   0  ran the boot pass (resumed 0..N confirmed-open repos, or nothing to do)
#   1  hard error (no gh)
#
# Environment:
#   AUTOSPEC_ACTIVE_RUNS_DIR     registry base dir (default ~/.autospec/active-runs)
#   AUTOSPEC_RUN_REGISTRY_SH     path to autospec-run-registry.sh (auto-resolved)
#   AUTOSPEC_RESUME_SCAN_SH      path to resume-scan.sh (auto-resolved); this is
#                                what `/autospec-resume --repo <r>` dispatches to
#   AUTOSPEC_SUPERVISOR_DRY_RUN  set to 1 to force --dry-run (no relaunch)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"

err()  { printf 'autospec-supervisor: %s\n' "$1" >&2; }
die()  { err "$1"; exit 1; }
say()  { printf '%s\n' "$1"; }

DRY_RUN=0
[ "${AUTOSPEC_SUPERVISOR_DRY_RUN:-0}" = "1" ] && DRY_RUN=1
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --help|-h) sed -n '2,30p' "$0"; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

command -v gh >/dev/null 2>&1 || die "gh CLI not found"

# ── Resolve sibling helpers (checkout layout or installed ~/.autospec/scripts). ─
resolve_helper() {
    override="$1"; shift
    if [ -n "$override" ] && [ -f "$override" ]; then printf '%s' "$override"; return; fi
    for c in "$@"; do
        [ -f "$c" ] && { printf '%s' "$c"; return; }
    done
    printf ''
}

RUN_REGISTRY_SH="$(resolve_helper "${AUTOSPEC_RUN_REGISTRY_SH:-}" \
    "$SCRIPT_DIR/autospec-run-registry.sh" \
    "$STATE_DIR/scripts/autospec-run-registry.sh")"
[ -n "$RUN_REGISTRY_SH" ] || die "autospec-run-registry.sh not found"

RESUME_SCAN_SH="$(resolve_helper "${AUTOSPEC_RESUME_SCAN_SH:-}" \
    "$SCRIPT_DIR/../skills/autospec-resume/scripts/resume-scan.sh" \
    "$STATE_DIR/scripts/resume-scan.sh")"
[ -n "$RESUME_SCAN_SH" ] || die "resume-scan.sh (/autospec-resume) not found"

# ── Independent GitHub confirmation: does <repo> have an OPEN in-progress run? ──
# This is the supervisor's own least-privilege gate. It does NOT trust the
# registry's resume_command — it asks GitHub directly. Only a confirmed-open run
# is handed to /autospec-resume.
confirm_open_run() {
    repo="$1"
    count="$(gh issue list --repo "$repo" --label in-progress-by-bot \
        --state open --json number --jq 'length' 2>/dev/null || echo 0)"
    case "$count" in
        ''|*[!0-9]*) return 1 ;;
    esac
    [ "$count" -ge 1 ]
}

# ── Iterate the registry. ──────────────────────────────────────────────────────
resumed=0
confirmed=0
registry_files="$(bash "$RUN_REGISTRY_SH" list 2>/dev/null || true)"

if [ -n "$registry_files" ]; then
    while IFS= read -r reg_file; do
        [ -n "$reg_file" ] || continue
        [ -f "$reg_file" ] || continue
        repo="$(jq -r '.repo // empty' "$reg_file" 2>/dev/null || true)"
        [ -n "$repo" ] || continue

        if ! confirm_open_run "$repo"; then
            # No independently-confirmed open run for this entry: do NOT relaunch.
            continue
        fi
        confirmed=$((confirmed + 1))

        if [ "$DRY_RUN" -eq 1 ]; then
            say "[dry-run] would resume $repo (confirmed open in-progress run)"
            resumed=$((resumed + 1))
            continue
        fi

        say "supervisor: resuming $repo (confirmed open in-progress run)"
        # Delegate to /autospec-resume (resume-scan.sh). It re-confirms
        # crash-vs-live via GitHub server updated_at and resolves the durable
        # resume_command itself; the supervisor never execs that command.
        bash "$RESUME_SCAN_SH" --repo "$repo" || \
            err "resume for $repo exited non-zero (continuing)"
        resumed=$((resumed + 1))
    done <<EOF
$registry_files
EOF
fi

# ── No confirmed open run anywhere -> exit 0, back off, do NOT re-arm. ─────────
if [ "$confirmed" -eq 0 ]; then
    say "no open run; nothing to resume (exit 0, backing off)"
    exit 0
fi

exit 0
