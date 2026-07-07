#!/usr/bin/env bash
# scripts/autonomous-waterfall.sh — Tier-selection decision helper.
#
# Evaluates tiers top-down per conductor cycle and emits a machine-readable
# JSON decision on stdout:
#   { "tier": N, "action": "<string>", "reason": "<string>" }
#
# Tiers:
#   Tier 0 — Control channel (preempts everything): a control label is present.
#   Tier 1 — Backlog (open auto-implement issues via gh).
#   Tier 2 — Local discovery: explore --once over local sources (spec-vs-code,
#             codebase-signals, source-analysis, dependency-health, prior-reports).
#             Phase-1 default: disabled; enable explicitly with
#             AUTOSPEC_ENABLE_DISCOVERY_TIERS=1.
#   Tier 3 — Internet discovery: explore --once --research-sources internet.
#             Phase-1 default: disabled; enable explicitly with
#             AUTOSPEC_ENABLE_DISCOVERY_TIERS=1.
#
# Dry-cycle escalation:
#   The caller tracks dry cycles per tier and passes --dry-cycles (Tier 1) and
#   --tier2-dry-cycles (Tier 2). A non-empty backlog floats selection back to
#   Tier 1 next cycle (dry counters are managed by the conductor).
#
# Usage:
#   autonomous-waterfall.sh [options]
#
# Options:
#   --control-decision LABEL   Active control label (e.g. autospec:pause).
#                              Empty string / omit = no control signal.
#   --dry-cycles N             Consecutive Tier-1 dry cycles so far (default 0).
#   --tier2-dry-cycles N       Consecutive Tier-2 dry cycles so far (default 0).
#   --repo OWNER/REPO          GitHub repo for backlog count (default: detect
#                              from git remote).
#   --backlog-count N          Inject backlog count directly (skips gh call;
#                              useful for testing).
#   -h, --help                 Print this help.
#
# Exit codes:
#   0  — decision emitted to stdout
#   2  — usage error

set -u

# ─── defaults ──────────────────────────────────────────────────────────────────
CONTROL_DECISION=""
DRY_CYCLES=0
TIER2_DRY_CYCLES=0
REPO=""
BACKLOG_COUNT_INJECT=""          # non-empty → skip gh call
DRY_CYCLES_THRESHOLD="${AUTOSPEC_AUTO_DRY_CYCLES:-2}"
DISCOVERY_TIERS_ENABLED="${AUTOSPEC_ENABLE_DISCOVERY_TIERS:-0}"

# ─── helpers ───────────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
Usage: autonomous-waterfall.sh [options]

Options:
  --control-decision LABEL   Active control label (autospec:pause, etc.).
  --dry-cycles N             Consecutive Tier-1 dry cycles (default 0).
  --tier2-dry-cycles N       Consecutive Tier-2 dry cycles (default 0).
  --repo OWNER/REPO          GitHub repo slug.
  --backlog-count N          Inject backlog count (bypasses gh; for testing).
  -h, --help                 Print this help.

Env:
  AUTOSPEC_AUTO_DRY_CYCLES   Dry-cycle escalation threshold (default 2).
  AUTOSPEC_ENABLE_DISCOVERY_TIERS
                              Set to 1 to allow Tier 2/3 discovery. Default 0
                              parks after the Tier-1 dry threshold in Phase 1.

Output (stdout):
  {"tier":<0|1|2|3>,"action":"<string>","reason":"<string>"}
EOF
}

emit() {
    local tier="$1" action="$2" reason="$3"
    # Minimal JSON — no external deps required.
    printf '{"tier":%s,"action":"%s","reason":"%s"}\n' "$tier" "$action" "$reason"
}

# ─── arg parsing ───────────────────────────────────────────────────────────────
while [ "$#" -gt 0 ]; do
    case "$1" in
        --control-decision)  CONTROL_DECISION="$2";    shift 2 ;;
        --dry-cycles)        DRY_CYCLES="$2";          shift 2 ;;
        --tier2-dry-cycles)  TIER2_DRY_CYCLES="$2";   shift 2 ;;
        --repo)              REPO="$2";                shift 2 ;;
        --backlog-count)     BACKLOG_COUNT_INJECT="$2"; shift 2 ;;
        -h|--help)           usage; exit 0 ;;
        *) printf 'autonomous-waterfall: unknown arg: %s\n' "$1" >&2; usage; exit 2 ;;
    esac
done

# ─── Tier 0 — Control channel (always preempts) ────────────────────────────────
if [ -n "$CONTROL_DECISION" ]; then
    emit 0 "control" "$CONTROL_DECISION"
    exit 0
fi

# ─── Tier 1 — Backlog: open auto-implement issues ──────────────────────────────
backlog_count=0

if [ -n "$BACKLOG_COUNT_INJECT" ]; then
    backlog_count="$BACKLOG_COUNT_INJECT"
else
    # Detect repo slug from git remote when not provided.
    if [ -z "$REPO" ]; then
        REPO="$(git remote get-url origin 2>/dev/null \
            | sed -E 's|.*github\.com[:/]||; s|\.git$||')" || true
    fi

    if [ -n "$REPO" ]; then
        # Count open issues with the auto-implement label.
        # gh may not be available in tests; treat failure as empty backlog.
        raw="$(gh issue list \
            --repo "$REPO" \
            --label auto-implement \
            --state open \
            --limit 1000 \
            --json number \
            --jq 'length' 2>/dev/null)" || raw=""
        backlog_count="${raw:-0}"
    fi
fi

if [ "$backlog_count" -gt 0 ] 2>/dev/null; then
    emit 1 "run-backlog" "backlog has $backlog_count open auto-implement issue(s)"
    exit 0
fi

# Backlog is empty — check Tier-1 dry-cycle threshold before escalating.
if [ "$DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 1 "run-backlog" "backlog empty but dry-cycles=$DRY_CYCLES < threshold=$DRY_CYCLES_THRESHOLD; staying at Tier 1"
    exit 0
fi

# Phase 1 only enables Tier 0 + Tier 1. Discovery tiers are an explicit opt-in
# until the Phase 2/3 discovery contract is intentionally activated.
if [ "$DISCOVERY_TIERS_ENABLED" != "1" ]; then
    emit 1 "park" "Tier 1 dry for $DRY_CYCLES cycle(s) >= threshold=$DRY_CYCLES_THRESHOLD; discovery tiers disabled in Phase 1"
    exit 0
fi

# ─── Tier 2 — Local discovery via explore --once ───────────────────────────────
# Tier 1 has been dry for >= threshold cycles. Check whether Tier 2 is also
# saturated before escalating to Tier 3.
if [ "$TIER2_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 2 "run-explore-once" "Tier 1 dry for $DRY_CYCLES cycle(s) >= threshold=$DRY_CYCLES_THRESHOLD; running local discovery (tier2-dry-cycles=$TIER2_DRY_CYCLES)"
    exit 0
fi

# ─── Tier 3 — Internet discovery via explore --once --research-sources internet ─
emit 3 "run-explore-once-internet" "Tier 2 dry for $TIER2_DRY_CYCLES cycle(s) >= threshold=$DRY_CYCLES_THRESHOLD; running internet discovery"
exit 0
