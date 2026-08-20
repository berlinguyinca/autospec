#!/usr/bin/env bash
# scripts/monitor-outer-loop.sh — deterministic outer-loop decision for the
# autospec Phase 4 monitor. Replaces the inline pseudocode in
# skills/autospec-run/SKILL.md (ready-queue scan, stop-sentinel check,
# ALL_DONE detection, issue start summary) with a single script invocation.
#
# The LLM monitor calls this script at the top of each outer-loop iteration
# and follows the emitted action. The script is deterministic: it performs
# no LLM judgment, no claim transitions, and no process(ISSUE) dispatch.
#
# Usage:
#   monitor-outer-loop.sh --repo <owner/name> \
#     [--batch-num <N>] [--batch-issue-count <N>] \
#     [--profile <name>]
#
# Output (stdout, one line):
#   action=<claim|sleep|all_done|stop> [key=value ...]
#
#   action=claim   issue=<N> title=<T> url=<U> labels=<L> goal=<G>
#                  smoke=<S> scope=<SC> body_file=<F>
#   action=sleep   reason=<R>
#   action=all_done batch=<N> processed=<N> repo=<R>
#   action=stop    mode=<M> batch=<N> processed=<N> repo=<R>
#
# Exit codes:
#   0  — action emitted successfully
#   1  — usage error or unexpected failure
#
# Dependencies: bash, gh, jq, awk, sed, head, date.
# The autospec binary is used for `queue ready` when available; the script
# falls back to `gh issue list` when the binary is absent.

set -eu

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
REPO=""
BATCH_NUM="${BATCH_NUM:-1}"
BATCH_ISSUE_COUNT="${BATCH_ISSUE_COUNT:-0}"
PROFILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) REPO="$2"; shift 2 ;;
    --batch-num) BATCH_NUM="$2"; shift 2 ;;
    --batch-issue-count) BATCH_ISSUE_COUNT="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    --help)
      sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      printf 'monitor-outer-loop.sh: error: unknown arg: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

if [ -z "$REPO" ]; then
  printf 'monitor-outer-loop.sh: error: --repo is required\n' >&2
  exit 1
fi

SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
AUTOSPEC_BIN="${AUTOSPEC_BIN:-autospec}"

# ---------------------------------------------------------------------------
# Emit helpers
# ---------------------------------------------------------------------------
emit_all_done() {
  # Write batch-done.json and diary entry, then emit the action line.
  if [ -f "$SCRIPTS_DIR/diary-write.sh" ]; then
    bash "$SCRIPTS_DIR/diary-write.sh" \
      --phase 4 --event monitor-exit \
      --body "Monitor ALL_DONE: batch=${BATCH_NUM} processed=${BATCH_ISSUE_COUNT} repo=${REPO}" \
      2>/dev/null || true
  fi
  printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
    "$BATCH_NUM" "$BATCH_ISSUE_COUNT" "$REPO" "$(date -u +%s)" \
    > "$HOME/.autospec/batch-done.json"
  printf 'action=all_done batch=%s processed=%s repo=%s\n' \
    "$BATCH_NUM" "$BATCH_ISSUE_COUNT" "$REPO"
}

emit_stop() {
  local mode="$1"
  if [ -f "$SCRIPTS_DIR/diary-write.sh" ]; then
    bash "$SCRIPTS_DIR/diary-write.sh" \
      --phase 4 --event monitor-exit \
      --body "Monitor stopped (${mode}): batch=${BATCH_NUM} processed=${BATCH_ISSUE_COUNT} repo=${REPO}" \
      2>/dev/null || true
  fi
  printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
    "$BATCH_NUM" "$BATCH_ISSUE_COUNT" "$REPO" "$(date -u +%s)" \
    > "$HOME/.autospec/batch-done.json"
  printf 'action=stop mode=%s batch=%s processed=%s repo=%s\n' \
    "$mode" "$BATCH_NUM" "$BATCH_ISSUE_COUNT" "$REPO"
}

emit_sleep() {
  local reason="$1"
  printf 'action=sleep reason=%s\n' "$reason"
}

# ---------------------------------------------------------------------------
# 1. Stop-sentinel check (outer loop, top of each iteration)
# ---------------------------------------------------------------------------
FLAG_FILE="$HOME/.autospec/stop.flag"
if [ -f "$FLAG_FILE" ]; then
  MODE="$(head -1 "$FLAG_FILE" 2>/dev/null || echo "")"
  TIMESTAMP="$(sed -n '2p' "$FLAG_FILE" 2>/dev/null | awk '{print $1}')"
  NOW="$(date -u +%s)"
  FLAG_TS="$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$TIMESTAMP" +%s 2>/dev/null \
    || date -u -d "$TIMESTAMP" +%s 2>/dev/null || echo 0)"
  AGE_SECS=$(( NOW - FLAG_TS ))
  if [ "$AGE_SECS" -gt 86400 ]; then
    printf 'WARN: stale stop.flag (%s s old); ignoring\n' "$AGE_SECS" >&2
  elif [ "$MODE" = "graceful" ] || [ "$MODE" = "immediate" ]; then
    emit_stop "$MODE"
    exit 0
  fi
fi

# ---------------------------------------------------------------------------
# 2. Ready-queue scan
# ---------------------------------------------------------------------------
# Use `autospec queue ready` when the binary is available; fall back to
# `gh issue list` otherwise. The queue-ready command returns JSON with a
# `ready` array (sorted by priority) and a `gate_counts` object.
# Write to a temp file to avoid storing the (potentially large) JSON in a
# shell variable.
QUEUE_FILE="$(mktemp "${TMPDIR:-/tmp}/autospec-queue.XXXXXX")"
trap 'rm -f "$QUEUE_FILE"' EXIT
QUEUE_OK=false
if command -v "$AUTOSPEC_BIN" >/dev/null 2>&1; then
  if "$AUTOSPEC_BIN" queue ready --repo "$REPO" --batch-size 1 >"$QUEUE_FILE" 2>/dev/null; then
    if jq -e '.ready' "$QUEUE_FILE" >/dev/null 2>&1; then
      QUEUE_OK=true
    fi
  fi
fi

if [ "$QUEUE_OK" = "true" ]; then
  READY_COUNT="$(jq '.ready | length' "$QUEUE_FILE")"
  OPEN_COUNT="$(jq '.gate_counts.open' "$QUEUE_FILE")"
  BLOCKED_COUNT="$(jq '.gate_counts.dependency_blocked // 0' "$QUEUE_FILE")"
else
  # Fallback: gh issue list
  rm -f "$QUEUE_FILE"
  OPEN_COUNT="$(gh issue list --repo "$REPO" --label auto-implement --state open \
    --json number 2>/dev/null | jq 'length' 2>/dev/null || echo 0)"
  READY_COUNT="$OPEN_COUNT"
  BLOCKED_COUNT=0
fi

# ---------------------------------------------------------------------------
# 3. ALL_DONE detection
# ---------------------------------------------------------------------------
if [ "${READY_COUNT:-0}" -eq 0 ]; then
  if [ "${OPEN_COUNT:-0}" -eq 0 ]; then
    # Check the most recent close time for any auto-implement issue.
    LATEST_CLOSE="$(gh issue list --repo "$REPO" --label auto-implement --state closed \
      --json closedAt --limit 1 2>/dev/null \
      | jq -r '.[0].closedAt // empty' 2>/dev/null || echo "")"
    IDLE=true
    if [ -n "$LATEST_CLOSE" ]; then
      CLOSE_TS="$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$LATEST_CLOSE" +%s 2>/dev/null \
        || date -u -d "$LATEST_CLOSE" +%s 2>/dev/null || echo 0)"
      NOW="$(date -u +%s)"
      if [ $(( NOW - CLOSE_TS )) -lt 7200 ]; then
        IDLE=false
      fi
    fi
    if [ "$IDLE" = "true" ]; then
      emit_all_done
      exit 0
    fi
  fi
  # Queue is empty but not ALL_DONE: sleep and retry.
  if [ "${BLOCKED_COUNT:-0}" -gt 0 ]; then
    emit_sleep "blocked: ${BLOCKED_COUNT} unmet deps"
  else
    emit_sleep "drained, waiting 2h idle"
  fi
  exit 0
fi

# ---------------------------------------------------------------------------
# 4. Emit the next ready issue's number
# ---------------------------------------------------------------------------
# The monitor uses issue-snapshot.sh to fetch the issue details (title, URL,
# labels, body) after claiming the issue. This script only emits the number.
if [ "$QUEUE_OK" = "true" ]; then
  ISSUE_NUM="$(jq -r '.ready[0].number' "$QUEUE_FILE")"
else
  # Fallback: fetch from gh
  ISSUE_NUM="$(gh issue list --repo "$REPO" --label auto-implement --state open \
    --json number --limit 1 2>/dev/null | jq -r '.[0].number')"
fi

printf 'action=claim issue=%s\n' "$ISSUE_NUM"

exit 0
