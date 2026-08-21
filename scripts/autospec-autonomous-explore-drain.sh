#!/usr/bin/env bash
# autospec-autonomous-explore-drain.sh — one discovery (explore --once) pass for
# the conductor, bridged through the active LLM harness.
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
# detected Codex, Claude, or OpenCode runtime, giving discovery a real
# orchestrator without crossing into an unrelated installed harness.
#
# filed/dry are derived from the count of `auto-implement` issues created during
# the run (the explore skill files via `gh issue create --label auto-implement`).
# All harness chatter goes to stderr; STDOUT carries exactly one JSON line.
#
# A failed/absent harness exits non-zero and emits `dry:false`; the conductor
# already maps that to a visible explore_error. Only a completed healthy scan
# may emit `dry:true`.
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
HARNESS_HELPER="$SCRIPT_DIR/lib/autospec-harness-detect.sh"
PROCESS_TREE_HELPER="$SCRIPT_DIR/lib/autospec-process-tree.sh"

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

emit_error() {
    printf '{"tier":"%s","proposals_seen":0,"new_candidates":0,"filed":0,"dry":false,"reason":"%s"}\n' \
        "$TIER" "${1:-explore-error}"
}

# Resolve the harness that owns this session. An explicit kind wins, followed
# by active runtime markers, then installed-home probes.
if [ ! -f "$HARNESS_HELPER" ]; then
    printf 'autospec-autonomous-explore-drain: harness detector missing: %s\n' "$HARNESS_HELPER" >&2
    emit_error "explore-error"
    exit 3
fi
# shellcheck source=/dev/null
. "$HARNESS_HELPER"
if [ ! -f "$PROCESS_TREE_HELPER" ]; then
    printf 'autospec-autonomous-explore-drain: process-tree helper missing: %s\n' "$PROCESS_TREE_HELPER" >&2
    emit_error "explore-error"
    exit 3
fi
# shellcheck source=/dev/null
. "$PROCESS_TREE_HELPER"
HARNESS_KIND="$(autospec_harness_detect)"
HARNESS_BINARY="$(autospec_harness_binary_for "$HARNESS_KIND" 2>/dev/null || true)"
HARNESS_DISPATCHER=""
if [ -n "$HARNESS_BINARY" ] && command -v "$HARNESS_BINARY" >/dev/null 2>&1; then
    HARNESS_DISPATCHER="$(command -v "$HARNESS_BINARY")"
fi
if [ -z "$HARNESS_DISPATCHER" ] || ! autospec_harness_dispatcher_safe "$HARNESS_DISPATCHER"; then
    printf 'autospec-autonomous-explore-drain: active harness unavailable or unsafe: %s\n' \
        "$HARNESS_KIND" >&2
    emit_error "explore-error"
    exit 3
fi
export AUTOSPEC_HANDOFF_DISPATCHER_KIND="$HARNESS_KIND"

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

# ── Build the harness-native explore skill invocation.
case "$HARNESS_KIND" in
    codex) SKILL_INVOCATION='$autospec-explore --once' ;;
    claude|opencode) SKILL_INVOCATION='/autospec-explore --once' ;;
    *) SKILL_INVOCATION='' ;;
esac
if [ -n "$RESEARCH_SOURCES" ]; then
    SKILL_INVOCATION="$SKILL_INVOCATION --research-sources $RESEARCH_SOURCES"
fi
VERIFY_CMD="${AUTOSPEC_EXPLORE_VERIFY_CMD:-bash $SCRIPT_DIR/autospec-autonomous-verify-drain.sh}"
export AUTOSPEC_EXPLORE_VERIFY_CMD="$VERIFY_CMD"

HARNESS_LOG="$(mktemp "${TMPDIR:-/tmp}/autospec-explore-drain.XXXXXX" 2>/dev/null || printf '/tmp/autospec-explore-drain.%s' "$$")"

run_in_new_session() {
    if command -v setsid >/dev/null 2>&1; then
        setsid "$@"
    else
        python3 -c 'import os, sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' "$@"
    fi
}

# Pass ownership across the harness boundary so a detached explore script can
# terminate itself when this drain is force-restarted or otherwise disappears.
export AUTOSPEC_EXPLORE_PARENT_PID="$$"

# Run explore through the harness. STDOUT+STDERR go to the log; the wrapper's own
# stdout is reserved for the single contract JSON line the conductor parses.
case "$HARNESS_KIND" in
    codex)
        run_in_new_session "$HARNESS_DISPATCHER" exec \
            --cd "$REPO_DIR" \
            --dangerously-bypass-approvals-and-sandbox \
            "$SKILL_INVOCATION" > "$HARNESS_LOG" 2>&1 &
        ;;
    claude)
        (
            cd "$REPO_DIR"
            run_in_new_session "$HARNESS_DISPATCHER" -p --dangerously-skip-permissions \
                "$SKILL_INVOCATION"
        ) > "$HARNESS_LOG" 2>&1 &
        ;;
    opencode)
        run_in_new_session "$HARNESS_DISPATCHER" run \
            --dir "$REPO_DIR" \
            --dangerously-skip-permissions \
            "$SKILL_INVOCATION" > "$HARNESS_LOG" 2>&1 &
        ;;
    *)
        printf 'autospec-autonomous-explore-drain: unsupported harness: %s\n' "$HARNESS_KIND" >&2
        emit_error "explore-error"
        exit 3
        ;;
esac
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
        now_epoch="$(date +%s)"
        total_secs=$((now_epoch - started_epoch))
        if [ "$total_secs" -ge "$EXPLORE_MAX_SECS" ]; then
            printf 'autospec-autonomous-explore-drain: max runtime %ss reached; terminating explore child pid %s\n' \
                "$EXPLORE_MAX_SECS" "$child_pid" >&2
            autospec_kill_tree "$child_pid" separate
            wait "$child_pid" 2>/dev/null || true
            explore_rc=124
            break
        fi
        current_size="$(stat -c '%s' "$HARNESS_LOG" 2>/dev/null || stat -f '%z' "$HARNESS_LOG" 2>/dev/null || printf '0')"
        if [ "$current_size" != "$last_size" ]; then
            last_size="$current_size"
            last_progress_epoch="$(date +%s)"
            continue
        fi
        idle_secs=$((now_epoch - last_progress_epoch))
        if [ "$idle_secs" -ge "$EXPLORE_STALL_SECS" ]; then
            printf 'autospec-autonomous-explore-drain: stalled after %ss with no output; terminating explore child pid %s\n' \
                "$EXPLORE_STALL_SECS" "$child_pid" >&2
            autospec_kill_tree "$child_pid" separate
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

# Some harness turns report a sealed verifier-no-op without actually invoking
# the supplied command. Bypass that model-reported dry result and execute the
# repository explore entrypoint directly so the verifier contract is real.
if grep -q 'AUTOSPEC_EXPLORE_VERIFY_CMD_not_executed' "$HARNESS_LOG" 2>/dev/null; then
    DIRECT_LOG="$(mktemp "${TMPDIR:-/tmp}/autospec-direct-explore.XXXXXX" 2>/dev/null || printf '/tmp/autospec-direct-explore.%s' "$$")"
    printf 'autospec-autonomous-explore-drain: harness skipped verifier; running direct explore fallback\n' >&2
    run_in_new_session env AUTOSPEC_EXPLORE_VERIFY_CMD="$VERIFY_CMD" AUTOSPEC_EXPLORE_AUTONOMOUS=1 \
        bash "$SCRIPT_DIR/autospec-explore.sh" --once >"$DIRECT_LOG" 2>&1 &
    direct_pid="$!"
    direct_started="$(date +%s)"
    direct_rc=0
    while kill -0 "$direct_pid" 2>/dev/null; do
        sleep "$EXPLORE_POLL_SECS"
        direct_elapsed=$(( $(date +%s) - direct_started ))
        if [ "$direct_elapsed" -ge "$EXPLORE_MAX_SECS" ]; then
            printf 'autospec-autonomous-explore-drain: direct fallback max runtime %ss reached; terminating pid %s\n' \
                "$EXPLORE_MAX_SECS" "$direct_pid" >&2
            autospec_kill_tree "$direct_pid" separate
            wait "$direct_pid" 2>/dev/null || true
            direct_rc=124
            break
        fi
    done
    if [ "$direct_rc" -eq 0 ]; then
        set +e
        wait "$direct_pid"
        direct_rc="$?"
        set -e
    fi
    cat "$DIRECT_LOG" >&2 2>/dev/null || true
    DIRECT_JSON="$(grep -E '^\{"tier"' "$DIRECT_LOG" | tail -1 || true)"
    rm -f "$DIRECT_LOG" "$HARNESS_LOG" 2>/dev/null || true
    if [ "$direct_rc" -ne 0 ]; then
        if [ -n "$DIRECT_JSON" ]; then
            printf '%s\n' "$DIRECT_JSON"
        else
            emit_error "explore-error"
        fi
        exit "$direct_rc"
    fi
    if [ -n "$DIRECT_JSON" ]; then
        printf '%s\n' "$DIRECT_JSON"
        exit 0
    fi
fi

# Surface harness output to stderr for observability (never to stdout).
cat "$HARNESS_LOG" >&2 2>/dev/null || true
rm -f "$HARNESS_LOG" 2>/dev/null || true

# A non-zero harness exit is incomplete discovery, never a clean dry.
if [ "$explore_rc" -ne 0 ]; then
    printf 'autospec-autonomous-explore-drain: explore harness exited %s\n' \
        "$explore_rc" >&2
    emit_error "explore-error"
    exit "$explore_rc"
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
