#!/usr/bin/env bash
# scripts/qa-heal-loop.sh — /autospec-qa self-healing loop orchestrator
# (issues #663 + #711).
#
# Default-on. Drives a discover → file → fix → re-QA cycle until convergence
# or a guardrail trips. Reads .autospec/qa-verdict.json after each round,
# translates release-blocking findings into auto-implement issues via
# qa-finding-to-issue.sh, dispatches /autospec-run to drain the queue,
# then re-enters the loop.
#
# Adopts the shared loop driver from scripts/lib/autospec-loop.sh (issue
# #708) for termination-condition naming: heal-loop status values are kept
# back-compat (convergence, oscillation_detected, round_cap_reached,
# budget_cap_reached, verify_first_violation, no_heal, stopped) AND the
# unified summary shape is mirrored to .autospec/qa-heal-summary.md so
# /autospec-release, /autospec-continue, /autospec-refine, and /autospec
# --loop all read structurally-identical loop summaries.
#
# Termination — loop exits when ANY of:
#   - convergence            zero release_blocking findings
#                            (== autospec-loop convergence_clean)
#   - oscillation_detected   round N+1 hash == round N hash
#   - round_cap_reached      $AUTOSPEC_HEAL_MAX_ROUNDS reached
#   - budget_cap_reached     token or wall-time cap exceeded
#   - evidence_based_stop    STOP: <reason> marker in QA report
#   - verify_first_violation same findings dropped as verified_dropped
#                            two rounds in a row
#   - stopped                ~/.autospec/stop.flag or qa-heal-stop.flag
#   - no_heal                --no-heal / --single-pass /
#                            ~/.autospec/{no-heal,qa-no-heal}.flag
#
# Exit codes:
#   0  — convergence (canonical success)
#   1  — non-convergence terminal state (operator review needed)
#   2  — usage / bad arguments
#
# Writes .autospec/heal-summary.md on every exit (including hard exits).

set -u

usage() {
    cat <<'EOF'
Usage: qa-heal-loop.sh [flags]

Flags:
  --no-heal               Discovery-only; do not file issues or loop.
  --single-pass           Alias for --no-heal (issue #711).
  --max-rounds N          Override AUTOSPEC_HEAL_MAX_ROUNDS (default 5).
  --token-cap N           Override AUTOSPEC_HEAL_TOKEN_CAP (default 1500000).
  --time-cap S            Override AUTOSPEC_HEAL_TIME_CAP (seconds, default 14400).
  --verdict PATH          Path to qa-verdict.json (default .autospec/qa-verdict.json).
  --summary PATH          Path to heal-summary.md (default .autospec/heal-summary.md).
  --simulate-tokens N     Test hook: pretend N tokens were used this round.
  --simulate-rounds DIR   Test hook: read round-N qa-verdict.json fixtures from DIR
                          rather than calling /autospec-qa. DIR/round-N.json.
  --help                  Show this and exit.

Environment:
  AUTOSPEC_HEAL_MAX_ROUNDS    default 5
  AUTOSPEC_HEAL_TOKEN_CAP     default 1500000
  AUTOSPEC_HEAL_TIME_CAP      default 14400 (seconds)
  ~/.autospec/no-heal.flag    forces --no-heal
  ~/.autospec/qa-no-heal.flag forces --no-heal (issue #711 alias)
  ~/.autospec/stop.flag       graceful or immediate stop sentinel
  ~/.autospec/qa-heal-stop.flag  per-skill stop sentinel (issue #711)
EOF
}

# Shared loop driver from PR #712 / issue #708. Sourced for the matcher
# library (extract-matchers.sh) and to register that qa-heal-loop is the
# heal-side adopter of the unified termination contract.
_AUTOSPEC_LOOP_LIB="$(cd "$(dirname "$0")" && pwd)/lib/autospec-loop.sh"
if [ -f "$_AUTOSPEC_LOOP_LIB" ]; then
    # shellcheck source=lib/autospec-loop.sh
    . "$_AUTOSPEC_LOOP_LIB" 2>/dev/null || true
fi

# Harness-aware loop dispatcher (issue #723). Used by run_autospec_drain
# when no AUTOSPEC_RUN_CMD test hook is set, so heal-loop's queue-drain step
# uses the correct per-harness `/autospec --autonomous` invocation form.
_AUTOSPEC_HARNESS_DETECT_LIB="$(cd "$(dirname "$0")" && pwd)/lib/autospec-harness-detect.sh"
if [ -f "$_AUTOSPEC_HARNESS_DETECT_LIB" ]; then
    # shellcheck source=lib/autospec-harness-detect.sh
    . "$_AUTOSPEC_HARNESS_DETECT_LIB" 2>/dev/null || true
fi

VERDICT=".autospec/qa-verdict.json"
SUMMARY=".autospec/heal-summary.md"
MAX_ROUNDS="${AUTOSPEC_HEAL_MAX_ROUNDS:-5}"
TOKEN_CAP="${AUTOSPEC_HEAL_TOKEN_CAP:-1500000}"
TIME_CAP="${AUTOSPEC_HEAL_TIME_CAP:-14400}"
NO_HEAL=0
SIM_TOKENS=""
SIM_ROUNDS=""

while [ $# -gt 0 ]; do
    case "$1" in
        --no-heal|--single-pass) NO_HEAL=1 ;;
        --max-rounds) MAX_ROUNDS="$2"; shift ;;
        --token-cap) TOKEN_CAP="$2"; shift ;;
        --time-cap)  TIME_CAP="$2";  shift ;;
        --verdict)   VERDICT="$2";   shift ;;
        --summary)   SUMMARY="$2";   shift ;;
        --simulate-tokens) SIM_TOKENS="$2"; shift ;;
        --simulate-rounds) SIM_ROUNDS="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "qa-heal-loop: unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [ -f "${HOME}/.autospec/no-heal.flag" ] \
    || [ -f "${HOME}/.autospec/qa-no-heal.flag" ]; then
    NO_HEAL=1
fi

# Hash a finding set deterministically: sort by (category, file, summary) and
# sha256 the result. Used for oscillation detection.
hash_findings() {
    local verdict_file="$1"
    if [ ! -f "$verdict_file" ]; then
        printf 'EMPTY\n'
        return
    fi
    jq -r '
        (.findings // [])
        | map(select(.release_blocking == true))
        | map([(.category // ""), (.file // ""), (.summary // "")] | join("|"))
        | sort
        | join("\n")
    ' "$verdict_file" 2>/dev/null | shasum -a 256 | awk '{print $1}'
}

count_blocking() {
    local verdict_file="$1"
    [ -f "$verdict_file" ] || { echo 0; return; }
    jq -r '(.findings // []) | map(select(.release_blocking == true)) | length' \
        "$verdict_file" 2>/dev/null || echo 0
}

count_verified_dropped() {
    local verdict_file="$1"
    [ -f "$verdict_file" ] || { echo 0; return; }
    jq -r '(.verified_dropped // []) | length' "$verdict_file" 2>/dev/null || echo 0
}

# Write the markdown summary table + persistent findings list.
write_summary() {
    local status="$1"
    local rounds_data="$2"   # one line per round: "N|findings|filed|merged|time"
    local persistent="$3"    # newline-separated category — summary lines
    local final_verdict="$4"
    mkdir -p "$(dirname "$SUMMARY")"
    {
        printf '## /autospec-qa heal loop summary\n\n'
        printf '| Round | Findings | Issues filed | PRs merged | Time | Status                  |\n'
        printf '|-------|---------:|-------------:|-----------:|------|-------------------------|\n'
        printf '%s\n' "$rounds_data" | while IFS='|' read -r r f filed merged t s; do
            [ -n "$r" ] || continue
            printf '| %-5s | %8s | %12s | %10s | %-4s | %-23s |\n' "$r" "$f" "$filed" "$merged" "$t" "$s"
        done
        printf '\nFinal verdict: %s\n' "$final_verdict"
        printf '\nExit status: %s\n' "$status"
        if [ -n "$persistent" ]; then
            printf '\n## Persistent findings (operator review needed)\n\n'
            printf '%s\n' "$persistent" | sed 's/^/- /'
        fi
    } > "$SUMMARY"
}

# Run one round of /autospec-qa or, in --simulate-rounds mode, copy the
# fixture for round N into place.
run_qa_round() {
    local round="$1"
    if [ -n "$SIM_ROUNDS" ]; then
        local fixture="$SIM_ROUNDS/round-${round}.json"
        if [ -f "$fixture" ]; then
            mkdir -p "$(dirname "$VERDICT")"
            cp "$fixture" "$VERDICT"
            return 0
        fi
        # No fixture for this round = no findings (convergence).
        mkdir -p "$(dirname "$VERDICT")"
        printf '{"verdict":"PASS","findings":[]}\n' > "$VERDICT"
        return 0
    fi
    # Real mode: defer to operator-supplied AUTOSPEC_QA_CMD or fail loudly.
    if [ -n "${AUTOSPEC_QA_CMD:-}" ]; then
        sh -c "$AUTOSPEC_QA_CMD"
        return $?
    fi
    echo "qa-heal-loop: no AUTOSPEC_QA_CMD set and no --simulate-rounds fixture" >&2
    return 2
}

# Dispatch /autospec-run to drain queue. Test hook: AUTOSPEC_RUN_CMD lets
# bats fixtures stub this. Returns count of PRs merged (best-effort).
run_autospec_drain() {
    local round="$1"
    if [ -n "${AUTOSPEC_RUN_CMD:-}" ]; then
        sh -c "$AUTOSPEC_RUN_CMD" 2>/dev/null
        return $?
    fi
    # Harness-aware dispatch (issue #723): when no AUTOSPEC_RUN_CMD test hook
    # is set, dispatch /autospec --autonomous through the per-harness binary
    # resolved by the shared detector.
    if declare -F autospec_harness_resolve_dispatcher >/dev/null 2>&1; then
        local _rc=0
        ( autospec_harness_resolve_dispatcher ) >/dev/null 2>&1 || _rc=$?
        if [ "$_rc" = 0 ]; then
            autospec_harness_resolve_dispatcher
            autospec_harness_invoke autonomous \
                "drain queued auto-implement issues for heal round $round" >/dev/null 2>&1
            return 0
        fi
    fi
    # No drain command and no harness binary → treat as 0 PRs merged.
    return 0
}

START_TS=$(date +%s)
RUNNING_TOKENS=0
ROUNDS_DATA=""
PERSISTENT=""
PREV_HASH=""
PREV_DROPPED_HASH=""
STATUS=""
FINAL_VERDICT="UNKNOWN"

if [ "$NO_HEAL" = 1 ]; then
    # Discovery only — single round, no issues, no run.
    run_qa_round 1 || true
    BLOCKING=$(count_blocking "$VERDICT")
    FINAL_VERDICT=$(jq -r '.verdict // "UNKNOWN"' "$VERDICT" 2>/dev/null || echo UNKNOWN)
    ROUNDS_DATA="1|${BLOCKING}|0|0|0m|no_heal (discovery only)"
    STATUS="no_heal"
    write_summary "$STATUS" "$ROUNDS_DATA" "" "$FINAL_VERDICT"
    echo "qa-heal-loop: $STATUS (verdict=$FINAL_VERDICT, blocking=$BLOCKING)"
    exit 0
fi

ROUND=0
while [ "$ROUND" -lt "$MAX_ROUNDS" ]; do
    ROUND=$((ROUND + 1))
    ROUND_START=$(date +%s)

    # Stop flag check at round boundary (issue #711: honor both global and
    # per-skill stop sentinels).
    if [ -f "${HOME}/.autospec/stop.flag" ] \
        || [ -f "${HOME}/.autospec/qa-heal-stop.flag" ]; then
        STATUS="stopped"
        break
    fi

    run_qa_round "$ROUND" || true
    BLOCKING=$(count_blocking "$VERDICT")
    # Evidence-based stop marker (issue #711): a verdict with
    # `stop_marker: "..."` or top-level `STOP: <reason>` field terminates
    # the loop with the unified evidence_based_stop status.
    STOP_MARKER=$(jq -r '.stop_marker // .STOP // empty' "$VERDICT" 2>/dev/null)
    if [ -n "$STOP_MARKER" ]; then
        ELAPSED=$(( $(date +%s) - ROUND_START ))
        ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|0|0|${ELAPSED}s|evidence_based_stop
"
        STATUS="evidence_based_stop"
        FINAL_VERDICT=$(jq -r '.verdict // "UNKNOWN"' "$VERDICT" 2>/dev/null || echo UNKNOWN)
        break
    fi
    HASH=$(hash_findings "$VERDICT")
    DROPPED_HASH=$(jq -r '(.verified_dropped // []) | map([(.category // ""), (.summary // "")] | join("|")) | sort | join("\n")' \
        "$VERDICT" 2>/dev/null | shasum -a 256 | awk '{print $1}')
    FINAL_VERDICT=$(jq -r '.verdict // "UNKNOWN"' "$VERDICT" 2>/dev/null || echo UNKNOWN)

    # Convergence — zero blocking findings.
    if [ "$BLOCKING" = 0 ]; then
        ELAPSED=$(( $(date +%s) - ROUND_START ))
        ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|0|0|${ELAPSED}s|convergence
"
        STATUS="convergence"
        break
    fi

    # Oscillation — finding-set hash unchanged round-over-round.
    if [ -n "$PREV_HASH" ] && [ "$HASH" = "$PREV_HASH" ]; then
        ELAPSED=$(( $(date +%s) - ROUND_START ))
        ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|0|0|${ELAPSED}s|oscillation_detected
"
        PERSISTENT=$(jq -r '
            (.findings // [])
            | map(select(.release_blocking == true))
            | map((.category // "?") + " — " + (.summary // "?"))
            | .[]
        ' "$VERDICT" 2>/dev/null)
        STATUS="oscillation_detected"
        break
    fi

    # Verify-first violation — same drop-set twice in a row indicates
    # the loop is fighting flaky probes.
    if [ -n "$PREV_DROPPED_HASH" ] && [ "$DROPPED_HASH" = "$PREV_DROPPED_HASH" ]; then
        DROPPED_COUNT=$(count_verified_dropped "$VERDICT")
        if [ "$DROPPED_COUNT" -gt 0 ]; then
            ELAPSED=$(( $(date +%s) - ROUND_START ))
            ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|0|0|${ELAPSED}s|verify_first_violation
"
            STATUS="verify_first_violation"
            break
        fi
    fi

    # File issues for each release_blocking finding. Bug-class sibling
    # sweep (issue #737): when findings carry a `parent_fingerprint`,
    # group them so the heal loop files ONE auto-implement issue per
    # pattern class — implementer fixes every matching instance in a
    # single PR. Findings without parent_fingerprint stay per-finding.
    FILED=0
    # Group 1: per-class (one issue per parent_fingerprint).
    while IFS= read -r class_hash; do
        [ -n "$class_hash" ] || continue
        class_finding=$(jq -c --arg fp "$class_hash" '
            (.findings // [])
            | map(select(.release_blocking == true))
            | map(select((.parent_fingerprint // "") == $fp
                         or (.fingerprint_hash // "") == $fp))
            | {
                category: "code_health:bug_class_sibling",
                release_blocking: true,
                parent_fingerprint: $fp,
                instances: map({file: .file, line: .line, confidence: (.confidence // 1.0)}),
                summary: ("bug-class sibling sweep: " + (length | tostring) + " instances of pattern " + $fp[0:12])
              }
        ' "$VERDICT" 2>/dev/null)
        [ -n "$class_finding" ] || continue
        if bash "$(dirname "$0")/qa-finding-to-issue.sh" --finding "$class_finding" >/dev/null 2>&1; then
            FILED=$((FILED + 1))
        fi
    done < <(jq -r '
        (.findings // [])
        | map(select(.release_blocking == true))
        | map(.parent_fingerprint // .fingerprint_hash // empty)
        | map(select(. != ""))
        | unique
        | .[]
    ' "$VERDICT" 2>/dev/null)
    # Group 2: ungrouped findings (no fingerprint) — per-finding as before.
    while IFS= read -r finding; do
        [ -n "$finding" ] || continue
        if bash "$(dirname "$0")/qa-finding-to-issue.sh" --finding "$finding" >/dev/null 2>&1; then
            FILED=$((FILED + 1))
        fi
    done < <(jq -c '
        (.findings // [])[]
        | select(.release_blocking == true)
        | select((.parent_fingerprint // "") == "" and (.fingerprint_hash // "") == "")
    ' "$VERDICT" 2>/dev/null)

    # Drain via /autospec-run.
    MERGED=$(run_autospec_drain "$ROUND")
    [ -z "$MERGED" ] && MERGED=0

    # Token + time caps.
    if [ -n "$SIM_TOKENS" ]; then
        RUNNING_TOKENS=$((RUNNING_TOKENS + SIM_TOKENS))
    fi
    if [ "$RUNNING_TOKENS" -gt "$TOKEN_CAP" ] 2>/dev/null; then
        ELAPSED=$(( $(date +%s) - ROUND_START ))
        ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|${FILED}|${MERGED}|${ELAPSED}s|budget_cap_reached
"
        STATUS="budget_cap_reached"
        break
    fi
    NOW=$(date +%s)
    if [ $((NOW - START_TS)) -gt "$TIME_CAP" ]; then
        ELAPSED=$(( NOW - ROUND_START ))
        ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|${FILED}|${MERGED}|${ELAPSED}s|budget_cap_reached
"
        STATUS="budget_cap_reached"
        break
    fi

    ELAPSED=$(( $(date +%s) - ROUND_START ))
    ROUNDS_DATA="${ROUNDS_DATA}${ROUND}|${BLOCKING}|${FILED}|${MERGED}|${ELAPSED}s|in_progress
"
    PREV_HASH="$HASH"
    PREV_DROPPED_HASH="$DROPPED_HASH"
done

if [ -z "$STATUS" ]; then
    STATUS="round_cap_reached"
    PERSISTENT=$(jq -r '
        (.findings // [])
        | map(select(.release_blocking == true))
        | map((.category // "?") + " — " + (.summary // "?"))
        | .[]
    ' "$VERDICT" 2>/dev/null)
fi

write_summary "$STATUS" "$ROUNDS_DATA" "$PERSISTENT" "$FINAL_VERDICT"

# Issue #711: mirror to .autospec/qa-heal-summary.md so the
# release/refine/continue loop summaries all live under predictable
# per-skill paths.
SUMMARY_DIR="$(dirname "$SUMMARY")"
if [ -f "$SUMMARY" ] && [ "$(basename "$SUMMARY")" != "qa-heal-summary.md" ]; then
    cp "$SUMMARY" "$SUMMARY_DIR/qa-heal-summary.md" 2>/dev/null || true
fi

echo "qa-heal-loop: $STATUS (verdict=$FINAL_VERDICT, rounds=$ROUND)"

case "$STATUS" in
    convergence) exit 0 ;;
    *)           exit 1 ;;
esac
