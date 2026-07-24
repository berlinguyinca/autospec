#!/usr/bin/env bash
# autospec-autonomous-explore-drain.sh — one discovery (explore --once) pass for
# the conductor, bridged through the LLM harness (omx).
#
# The conductor loop (scripts/lib/autospec-loop.sh) resolves Tier-2/3/4 discovery
# from AUTOSPEC_EXPLORE_CMD and appends the flags `--once` and, for Tier 4,
# `--research-sources internet`. It parses this wrapper's STDOUT as the explore
# yield contract (see tests/autonomous/test_sandbox_routing.bats):
#
#   {"tier":"local|competitor","proposals_seen":N,"new_candidates":N,
#    "filed":N,"dry":<bool>,"reason":"..."}
#
# Bare `bash autospec-explore.sh --once` has no LLM orchestrator to dispatch the
# researcher subagents and the fail-closed adversarial verify, so every proposal
# is refused and every cycle reports dry. This wrapper runs the explore SKILL via
# `omx exec` (mirroring autospec-autonomous-run-drain.sh's `$autospec-run` call),
# giving discovery a real orchestrator.
#
# filed/dry are derived from the count of `auto-implement` issues created during
# the run (the explore skill files via `gh issue create --label auto-implement`).
# All harness chatter goes to stderr; STDOUT carries exactly one JSON line.
#
# A failed/absent harness is a CLEAN DRY (exit 0), never a conductor crash: the
# loop treats a non-zero wrapper exit as a visible explore_error, but the wrapper
# itself must not abort the conductor.
set -eu

# The explore model can rediscover the conductor's helper command while
# following its own runbook. Inherited marker state makes that recursion
# explicit and fail-closed instead of spawning an unbounded harness tree.
if [ "${AUTOSPEC_EXPLORE_DRAIN_ACTIVE:-0}" = "1" ]; then
    printf '{"tier":"local","proposals_seen":0,"new_candidates":0,"filed":0,"dry":true,"reason":"nested-explore-suppressed"}\n'
    exit 0
fi
export AUTOSPEC_EXPLORE_DRAIN_ACTIVE=1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Parse the flags the conductor appends. ────────────────────────────────────
RESEARCH_SOURCES=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --once)             shift ;;
        --research-sources) RESEARCH_SOURCES="${2:-}"; shift 2 ;;
        *)                  shift ;;
    esac
done

# Tier label mirrors autospec-explore.sh --once: internet sources → competitor.
TIER="local"
case ",${RESEARCH_SOURCES}," in
    *,internet,*) TIER="competitor" ;;
esac

emit_dry() {
    # $1 = reason
    printf '{"tier":"%s","proposals_seen":0,"new_candidates":0,"filed":0,"dry":true,"reason":"%s"}\n' \
        "$TIER" "${1:-explore-dry}"
}

# ── Harness absence is a clean dry (never hard-fail the conductor). ───────────
if ! command -v omx >/dev/null 2>&1; then
    printf 'autospec-autonomous-explore-drain: omx not found on PATH; reporting clean dry\n' >&2
    emit_dry "explore-error"
    exit 0
fi

if [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

DEFAULT_REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if command -v autospec_runtime_config_path >/dev/null 2>&1; then
    REPO_DIR="$(autospec_runtime_config_path autonomous.repo_dir AUTOSPEC_REPO_DIR "$DEFAULT_REPO_DIR")"
else
    REPO_DIR="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
fi
if command -v autospec_runtime_config_int >/dev/null 2>&1; then
    EXPLORE_STALL_SECS="$(autospec_runtime_config_int autonomous.explore.stall_secs AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS 120)"
    EXPLORE_MAX_SECS="$(autospec_runtime_config_int autonomous.explore.max_secs AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS 300)"
    EXPLORE_POLL_SECS="$(autospec_runtime_config_int autonomous.explore.poll_secs AUTOSPEC_AUTONOMOUS_EXPLORE_POLL_SECS 15)"
else
    EXPLORE_STALL_SECS="${AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS:-120}"
    EXPLORE_MAX_SECS="${AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS:-300}"
    EXPLORE_POLL_SECS="${AUTOSPEC_AUTONOMOUS_EXPLORE_POLL_SECS:-15}"
fi

kill_tree() {
    _pid="$1"
    _pgid="$(ps -o pgid= -p "$_pid" 2>/dev/null | tr -d ' ' || true)"
    _own_pgid="$(ps -o pgid= -p "$$" 2>/dev/null | tr -d ' ' || true)"
    if [ -n "$_pgid" ] && [ "$_pgid" != "$_own_pgid" ]; then
        kill -TERM -- "-$_pgid" 2>/dev/null || true
        kill -KILL -- "-$_pgid" 2>/dev/null || true
        return 0
    fi
    for _child in $(pgrep -P "$_pid" 2>/dev/null || true); do
        kill_tree "$_child"
    done
    kill "$_pid" 2>/dev/null || true
}

detect_repo() {
    if [ -n "${CONDUCTOR_REPO:-}" ]; then
        printf '%s\n' "$CONDUCTOR_REPO"
        return 0
    fi
    if [ -n "${AUTOSPEC_REPO:-}" ]; then
        printf '%s\n' "$AUTOSPEC_REPO"
        return 0
    fi
    gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true
}

# Count open auto-implement issues — the signal the explore skill mutates by
# filing discovery candidates. Empty string when gh/repo are unavailable.
count_auto_issues() {
    _repo="$1"
    command -v gh >/dev/null 2>&1 || { printf ''; return 0; }
    [ -n "$_repo" ] || { printf ''; return 0; }
    gh issue list \
        --repo "$_repo" \
        --label auto-implement \
        --state open \
        --limit 1000 \
        --json number \
        --jq 'length' 2>/dev/null || printf ''
}

REPO="$(detect_repo)"
BEFORE="$(count_auto_issues "$REPO")"

# ── Build the explore skill invocation (mirrors the drain wrapper's $autospec-run).
SKILL_INVOCATION='$autospec-explore --once'
if [ -n "$RESEARCH_SOURCES" ]; then
    SKILL_INVOCATION="$SKILL_INVOCATION --research-sources $RESEARCH_SOURCES"
fi
# `omx exec` may sanitize exported environment variables. Carry the verifier
# bridge in the command itself so the nested explore process cannot silently
# enter its fail-closed no-verifier path.
VERIFY_CMD="${AUTOSPEC_EXPLORE_VERIFY_CMD:-bash $SCRIPT_DIR/autospec-autonomous-verify-drain.sh}"
printf -v VERIFY_ASSIGNMENT 'AUTOSPEC_EXPLORE_VERIFY_CMD=%q' "$VERIFY_CMD"
SKILL_INVOCATION="$VERIFY_ASSIGNMENT $SKILL_INVOCATION"

HARNESS_LOG="$(mktemp "${TMPDIR:-/tmp}/autospec-explore-drain.XXXXXX" 2>/dev/null || printf '/tmp/autospec-explore-drain.%s' "$$")"

# Pass ownership across the harness boundary so a detached explore script can
# terminate itself when this drain is force-restarted or otherwise disappears.
export AUTOSPEC_EXPLORE_PARENT_PID="$$"

# Run explore through the harness. STDOUT+STDERR go to the log; the wrapper's own
# stdout is reserved for the single contract JSON line the conductor parses.
setsid omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    "$SKILL_INVOCATION" > "$HARNESS_LOG" 2>&1 &
child_pid="$!"

explore_rc=0
if [ "${EXPLORE_STALL_SECS:-0}" -le 0 ] 2>/dev/null; then
    # No stall watchdog — plain wait.
    set +e
    wait "$child_pid"
    explore_rc="$?"
    set -e
else
    last_size=0
    last_progress_epoch="$(date +%s)"
    started_epoch="$last_progress_epoch"
    while kill -0 "$child_pid" 2>/dev/null; do
        sleep "$EXPLORE_POLL_SECS"
        current_size="$(stat -c '%s' "$HARNESS_LOG" 2>/dev/null || stat -f '%z' "$HARNESS_LOG" 2>/dev/null || printf '0')"
        if [ "$current_size" != "$last_size" ]; then
            last_size="$current_size"
            last_progress_epoch="$(date +%s)"
            continue
        fi
        now_epoch="$(date +%s)"
        total_secs=$((now_epoch - started_epoch))
        if [ "$total_secs" -ge "$EXPLORE_MAX_SECS" ]; then
            printf 'autospec-autonomous-explore-drain: max runtime %ss reached; terminating explore child pid %s\n' \
                "$EXPLORE_MAX_SECS" "$child_pid" >&2
            kill_tree "$child_pid"
            wait "$child_pid" 2>/dev/null || true
            explore_rc=124
            break
        fi
        idle_secs=$((now_epoch - last_progress_epoch))
        if [ "$idle_secs" -ge "$EXPLORE_STALL_SECS" ]; then
            printf 'autospec-autonomous-explore-drain: stalled after %ss with no output; terminating explore child pid %s\n' \
                "$EXPLORE_STALL_SECS" "$child_pid" >&2
            kill_tree "$child_pid"
            wait "$child_pid" 2>/dev/null || true
            explore_rc=124
            break
        fi
    done
    if [ "$explore_rc" -eq 0 ]; then
        set +e
        wait "$child_pid"
        explore_rc="$?"
        set -e
    fi
fi

# Surface harness output to stderr for observability (never to stdout).
cat "$HARNESS_LOG" >&2 2>/dev/null || true
rm -f "$HARNESS_LOG" 2>/dev/null || true

# A non-zero harness exit (crash, stall, misconfig) is a clean dry to the
# conductor's stdout parser, tagged explore-error — the loop surfaces the
# distinct code_health signal from the wrapper's non-zero-less contract.
if [ "$explore_rc" -ne 0 ]; then
    printf 'autospec-autonomous-explore-drain: explore harness exited %s; reporting clean dry\n' \
        "$explore_rc" >&2
    emit_dry "explore-error"
    exit 0
fi

AFTER="$(count_auto_issues "$REPO")"

# Derive filed from the auto-implement issue count delta across the run.
FILED=0
if [ -n "$BEFORE" ] && [ -n "$AFTER" ]; then
    case "$BEFORE$AFTER" in
        *[!0-9]*) : ;;
        *)
            _delta=$((AFTER - BEFORE))
            [ "$_delta" -gt 0 ] && FILED="$_delta"
            ;;
    esac
fi

if [ "$FILED" -gt 0 ]; then
    printf '{"tier":"%s","proposals_seen":%s,"new_candidates":%s,"filed":%s,"dry":false,"reason":"filed %s auto-implement issue(s) via LLM-bridged explore --once"}\n' \
        "$TIER" "$FILED" "$FILED" "$FILED" "$FILED"
else
    emit_dry "no new candidates after LLM-bridged explore --once"
fi
exit 0
