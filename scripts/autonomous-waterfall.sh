#!/usr/bin/env bash
# scripts/autonomous-waterfall.sh — Tier-selection decision helper.
#
# Evaluates tiers top-down per conductor cycle and emits a machine-readable
# JSON decision on stdout:
#   { "tier": N, "action": "<string>", "reason": "<string>" }
#
# Tiers:
#   Tier 0   — Control channel (preempts everything): a control label is present.
#   Tier 1   — Backlog (open auto-implement issues via gh).
#   Tier 1.5 — Promote/decompose/classify existing open issues into the pipeline.
#   Tier 2   — Local discovery: explore --once over local sources.
#   Tier 3   — Architecture, test-coverage, and technical-debt improvement.
#   Tier 4   — Internet/operator-polish discovery.
#
# Dry-cycle escalation:
#   The caller tracks dry cycles per tier and passes them in. A non-empty backlog
#   always floats selection back to Tier 1.

set -u

CONTROL_DECISION=""
DRY_CYCLES=0
TIER15_DRY_CYCLES=0
TIER2_DRY_CYCLES=0
TIER3_DRY_CYCLES=0
TIER4_DRY_CYCLES=0
REPO=""
BACKLOG_COUNT_INJECT=""
OPEN_ISSUE_COUNT_INJECT=""
CANDIDATE_FILE="${AUTOSPEC_PRIORITY_CANDIDATES:-}"
RECENT_TOUCHES_FILE="${AUTOSPEC_RECENT_TOUCHES:-}"
VALUE_FLOOR="${AUTOSPEC_VALUE_FLOOR:-1}"
RECENT_DECAY="${AUTOSPEC_RECENT_DECAY:-0.5}"
HUMAN_GATE_BLAST_RADIUS="${AUTOSPEC_HUMAN_GATE_BLAST_RADIUS:-4}"
PRIORITY_QUEUE_OUT="${AUTOSPEC_PRIORITY_QUEUE_OUT:-}"
DRY_CYCLES_THRESHOLD="${AUTOSPEC_AUTO_DRY_CYCLES:-2}"
DISCOVERY_TIERS_DISABLED="${AUTOSPEC_DISABLE_DISCOVERY_TIERS:-0}"
GROWTH_ENABLED="${AUTOSPEC_GROWTH_ENABLED:-0}"
GROWTH_OUTBOUND_PENDING=""
GROWTH_BACKLOG=""
GROWTH_BACKLOG_FLOOR="${AUTOSPEC_GROWTH_BACKLOG_FLOOR:-3}"
GROWTH_MEASURE_DUE="0"
TIERG_DRY_CYCLES=0

usage() {
    cat <<'USAGE'
Usage: autonomous-waterfall.sh [options]

Options:
  --control-decision LABEL    Active control label (autospec:pause, etc.).
  --dry-cycles N              Consecutive Tier-1 dry cycles (default 0).
  --tier15-dry-cycles N       Consecutive Tier-1.5 dry cycles (default 0).
  --tier2-dry-cycles N        Consecutive Tier-2 dry cycles (default 0).
  --tier3-dry-cycles N        Consecutive Tier-3 dry cycles (default 0).
  --tier4-dry-cycles N        Consecutive Tier-4 dry cycles (default 0).
  --repo OWNER/REPO           GitHub repo slug.
  --backlog-count N           Inject auto-implement backlog count (for testing).
  --open-issue-count N        Inject non-auto open issue count (for testing).
  --candidate-file FILE       Candidate JSON/JSONL queue to value-score.
  --recent-touches FILE       Recent touch JSON/JSONL for anti-thrash decay.
  --value-floor N             Idle when top score is below this floor (default 1).
  --recent-decay N            Recent-touch multiplier (default 0.5).
  --human-gate-blast-radius N Route at/above this blast radius to human gate.
  --growth-enabled 0|1        Enable capability-gated GROWTH tiers 5-7 (default 0).
  --growth-outbound-pending N Pending outbound draft/approval item count.
  --growth-backlog N          Current growth-artifact backlog count.
  --growth-backlog-floor M    Backlog floor below which growth define runs (default 3).
  --growth-measure-due 0|1    Growth measure interval elapsed.
  --tierg-dry-cycles N        Consecutive growth-define dry cycles (default 0).
  -h, --help                  Print this help.

Env:
  AUTOSPEC_AUTO_DRY_CYCLES        Dry-cycle escalation threshold (default 2).
  AUTOSPEC_DISABLE_DISCOVERY_TIERS Set to 1 for an emergency fail-closed park at
                                  the Tier-1 dry threshold.
  AUTOSPEC_PRIORITY_CANDIDATES     Candidate JSON/JSONL file for value-gated scoring.
  AUTOSPEC_VALUE_FLOOR             Priority floor below which conductor idles.
  AUTOSPEC_GROWTH_ENABLED          Enable capability-gated GROWTH tiers 5-7 (default 0).
  AUTOSPEC_GROWTH_BACKLOG_FLOOR    Growth backlog floor (default 3).

Output (stdout):
  {"tier":<0|1|1.5|2|3|4|5|6|7>,"action":"<string>","reason":"<string>"}
USAGE
}

emit() {
    local tier="$1" action="$2" reason="$3"
    printf '{"tier":%s,"action":"%s","reason":"%s"}\n' "$tier" "$action" "$reason"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --control-decision)   CONTROL_DECISION="$2"; shift 2 ;;
        --dry-cycles)         DRY_CYCLES="$2"; shift 2 ;;
        --tier15-dry-cycles)  TIER15_DRY_CYCLES="$2"; shift 2 ;;
        --tier2-dry-cycles)   TIER2_DRY_CYCLES="$2"; shift 2 ;;
        --tier3-dry-cycles)   TIER3_DRY_CYCLES="$2"; shift 2 ;;
        --tier4-dry-cycles)   TIER4_DRY_CYCLES="$2"; shift 2 ;;
        --repo)               REPO="$2"; shift 2 ;;
        --backlog-count)      BACKLOG_COUNT_INJECT="$2"; shift 2 ;;
        --open-issue-count)   OPEN_ISSUE_COUNT_INJECT="$2"; shift 2 ;;
        --candidate-file)     CANDIDATE_FILE="$2"; shift 2 ;;
        --recent-touches)     RECENT_TOUCHES_FILE="$2"; shift 2 ;;
        --value-floor)        VALUE_FLOOR="$2"; shift 2 ;;
        --recent-decay)       RECENT_DECAY="$2"; shift 2 ;;
        --human-gate-blast-radius) HUMAN_GATE_BLAST_RADIUS="$2"; shift 2 ;;
        --growth-enabled)          GROWTH_ENABLED="$2"; shift 2 ;;
        --growth-outbound-pending) GROWTH_OUTBOUND_PENDING="$2"; shift 2 ;;
        --growth-backlog)          GROWTH_BACKLOG="$2"; shift 2 ;;
        --growth-backlog-floor)    GROWTH_BACKLOG_FLOOR="$2"; shift 2 ;;
        --growth-measure-due)      GROWTH_MEASURE_DUE="$2"; shift 2 ;;
        --tierg-dry-cycles)        TIERG_DRY_CYCLES="$2"; shift 2 ;;
        -h|--help)            usage; exit 0 ;;
        *) printf 'autonomous-waterfall: unknown arg: %s\n' "$1" >&2; usage; exit 2 ;;
    esac
done

if [ -n "$CONTROL_DECISION" ]; then
    emit 0 "control" "$CONTROL_DECISION"
    exit 0
fi

# Optional value-gated cross-workstream scorer. When a candidate file is supplied,
# it becomes the top-level arbiter: run the highest-value safe item, idle below
# the value floor, or route fenced/high-blast-radius work to a human gate.
if [ -n "$CANDIDATE_FILE" ] && [ -s "$CANDIDATE_FILE" ]; then
    _prioritize="${AUTOSPEC_PRIORITIZE_BIN:-}"
    if [ -z "$_prioritize" ] && [ -f "$(dirname "$0")/autonomous-prioritize.sh" ]; then
        _prioritize="$(dirname "$0")/autonomous-prioritize.sh"
    fi
    if [ -n "$_prioritize" ]; then
        _prio_cmd=(bash "$_prioritize" score --candidates "$CANDIDATE_FILE" --value-floor "$VALUE_FLOOR" --recent-decay "$RECENT_DECAY" --human-gate-blast-radius "$HUMAN_GATE_BLAST_RADIUS")
        if [ -n "$RECENT_TOUCHES_FILE" ]; then
            _prio_cmd+=(--recent-touches "$RECENT_TOUCHES_FILE")
        fi
        if [ -n "$PRIORITY_QUEUE_OUT" ]; then
            _prio_cmd+=(--out "$PRIORITY_QUEUE_OUT")
        fi
        _prio_json="$("${_prio_cmd[@]}" 2>/dev/null || true)"
        _prio_decision="$(printf '%s' "$_prio_json" | jq -r '.decision // empty' 2>/dev/null || true)"
        _prio_top="$(printf '%s' "$_prio_json" | jq -r '.top.id // "none"' 2>/dev/null || echo none)"
        _prio_score="$(printf '%s' "$_prio_json" | jq -r '.top.score // 0' 2>/dev/null || echo 0)"
        case "$_prio_decision" in
            human_gate)
                emit 0 "control" "human-gate: value queue selected fenced/high-blast-radius candidate ${_prio_top}; route to human gate"
                exit 0
                ;;
            idle)
                emit 1 "run-backlog" "idle-rescan: highest candidate score ${_prio_score} below value floor ${VALUE_FLOOR}; idle and re-scan"
                exit 0
                ;;
            run)
                emit 1 "run-backlog" "value queue selected candidate ${_prio_top} score ${_prio_score}"
                exit 0
                ;;
        esac
    fi
fi

if [ -z "$REPO" ]; then
    REPO="$(git remote get-url origin 2>/dev/null \
        | sed -E 's|.*github\.com[:/]||; s|\.git$||')" || true
fi

backlog_count=0
if [ -n "$BACKLOG_COUNT_INJECT" ]; then
    # Primary path (#1632): the caller (lib/autospec-loop.sh) already computed
    # the dependency-aware ready count via autospec queue ready — trust it.
    backlog_count="$BACKLOG_COUNT_INJECT"
else
    # Defense-in-depth (#1632): no injected count — this is a standalone
    # invocation (tests, other callers). Prefer the dependency-aware
    # Rust queue (ready+batch) over the naive open-issue-count `gh`
    # query, so a fully-blocked backlog yields backlog_count=0 here too — one
    # readiness definition, matching the drain. Fall back to the naive `gh`
    # count only when the Rust queue command is unavailable.
    _queue_bin="${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-}}"
    if [ -z "$_queue_bin" ]; then
        _wf_dir="$(cd "$(dirname "$0")" && pwd)"
        if [ -x "$_wf_dir/../target/debug/autospec" ]; then
            _queue_bin="$_wf_dir/../target/debug/autospec"
        elif command -v autospec >/dev/null 2>&1; then
            _queue_bin="$(command -v autospec)"
        fi
    fi
    if [ -n "$_queue_bin" ] && { [ -x "$_queue_bin" ] || command -v "$_queue_bin" >/dev/null 2>&1; } && [ -n "$REPO" ]; then
        _ready_json="$("$_queue_bin" queue ready --repo "$REPO" --batch-size 1 2>/dev/null)"
        backlog_count="$(printf '%s' "$_ready_json" \
            | jq -r 'if (.worker_cap.reached // false) then 0 else ((.ready|length) + (.batch|length)) end' \
              2>/dev/null)"
        case "$backlog_count" in *[!0-9]*|'') backlog_count="" ;; esac
    fi
    if [ -z "$backlog_count" ] && [ -n "$REPO" ]; then
        raw="$(gh issue list \
            --repo "$REPO" \
            --label auto-implement \
            --state open \
            --limit 1000 \
            --json number,labels \
            --jq '[.[] | select([.labels[].name] | index("autospec:run-accountability") | not)] | length' 2>/dev/null)" || raw=""
        backlog_count="${raw:-0}"
    fi
    backlog_count="${backlog_count:-0}"
fi

if [ "$backlog_count" -gt 0 ] 2>/dev/null; then
    emit 1 "run-backlog" "backlog has $backlog_count open auto-implement issue(s)"
    exit 0
fi

if [ "$DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 1 "run-backlog" "backlog empty but dry-cycles=$DRY_CYCLES < threshold=$DRY_CYCLES_THRESHOLD; staying at Tier 1"
    exit 0
fi

if [ "$DISCOVERY_TIERS_DISABLED" = "1" ]; then
    emit 1 "park" "Tier 1 dry for $DRY_CYCLES cycle(s) >= threshold=$DRY_CYCLES_THRESHOLD; discovery tiers disabled by AUTOSPEC_DISABLE_DISCOVERY_TIERS"
    exit 0
fi

open_issue_count=0
if [ -n "$OPEN_ISSUE_COUNT_INJECT" ]; then
    open_issue_count="$OPEN_ISSUE_COUNT_INJECT"
elif [ -n "$REPO" ]; then
    raw="$(gh issue list \
        --repo "$REPO" \
        --state open \
        --limit 1000 \
        --json number,labels \
        --jq '[.[] | select(([.labels[].name] | index("auto-implement") | not) and ([.labels[].name] | index("paused-by-user") | not) and ([.labels[].name] | index("locked-by-autospec-processor") | not) and ([.labels[].name] | index("autospec:run-accountability") | not))] | length' \
        2>/dev/null)" || raw=""
    open_issue_count="${raw:-0}"
fi

if [ "$open_issue_count" -gt 0 ] 2>/dev/null && \
        [ "$TIER15_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 1.5 "promote-open-issues" "Tier 1 dry and $open_issue_count non-auto open issue(s) available; promoting/decomposing/classifying existing issues"
    exit 0
fi

if [ "$TIER2_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 2 "run-explore-once" "Tier 1/Tier 1.5 dry; running local discovery (tier2-dry-cycles=$TIER2_DRY_CYCLES)"
    exit 0
fi

if [ "$TIER3_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 3 "run-architecture-improvement" "Tier 2 dry; generating architecture/test-coverage improvement work (tier3-dry-cycles=$TIER3_DRY_CYCLES)"
    exit 0
fi

if [ "$TIER4_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 4 "run-explore-once-internet" "Tier 3 dry; running internet/operator-polish discovery (tier4-dry-cycles=$TIER4_DRY_CYCLES)"
    exit 0
fi

# ── Growth tiers (capability-gated on .autospec/growth.yml; the caller sets
# --growth-enabled). Appended after Tier 4 so Tiers 0–4 keep their numbers.
# growth:artifact IMPLEMENTATION already competes in Tier 1 (auto-implement);
# these tiers are the growth META-work (outbound service, candidate research,
# measurement) that fills otherwise-idle cycles.
if [ "$GROWTH_ENABLED" = "1" ]; then
    if [ -n "$GROWTH_OUTBOUND_PENDING" ] && [ "$GROWTH_OUTBOUND_PENDING" -gt 0 ] 2>/dev/null; then
        emit 5 "service-growth-outbound" "growth: $GROWTH_OUTBOUND_PENDING outbound draft/approval item(s) pending"
        exit 0
    fi
    if [ -n "$GROWTH_BACKLOG" ] && \
            [ "$GROWTH_BACKLOG" -lt "$GROWTH_BACKLOG_FLOOR" ] 2>/dev/null && \
            [ "$TIERG_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
        emit 6 "run-growth-define" "growth: artifact backlog=$GROWTH_BACKLOG below floor=$GROWTH_BACKLOG_FLOOR; researching candidates (tierg-dry-cycles=$TIERG_DRY_CYCLES)"
        exit 0
    fi
    if [ "$GROWTH_MEASURE_DUE" = "1" ]; then
        emit 7 "run-growth-measure" "growth: measure interval elapsed; measuring and re-weighting"
        exit 0
    fi
fi

# Never-idle contract (docs/specs/2026-07-06-autospec-autonomous-platform-design.md,
# R1/R5, F1): a fully-dry cascade must NOT convergence-park. It enters idle-rescan
# — the conductor arms resume and re-scans after AUTOSPEC_RESCAN_INTERVAL, keeping
# the loop alive. Only resource/control park (spend ledger, usage governor, control
# labels) or the AUTOSPEC_DISABLE_DISCOVERY_TIERS emergency kill-switch above may
# exit; convergence-stop is forbidden.
emit 4 "idle-rescan" "all tiers dry; idle and re-scan: Tier 1 backlog, Tier 1.5 open-issue promotion, Tier 2 local discovery, Tier 3 architecture/test coverage, and Tier 4 internet/operator discovery exhausted"
exit 0
