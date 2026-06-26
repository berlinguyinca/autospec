#!/usr/bin/env bash
# scripts/autonomous-waterfall.sh — Phase-1 tier-selection decision helper.
#
# Evaluates tiers top-down per conductor cycle and emits a machine-readable
# JSON decision on stdout:
#   { "tier": N, "action": "<string>", "reason": "<string>" }
#
# Phase-1 scope (only tiers 0 and 1 are enabled):
#   Tier 0 — Control channel (preempts everything): a control label is present.
#   Tier 1 — Backlog (open auto-implement issues via gh).
#   Tiers 2–4 — Discovery/polish: NOT YET ENABLED; always print not-enabled.
#
# Dry-cycle escalation: if the backlog is empty for < AUTOSPEC_AUTO_DRY_CYCLES
# consecutive cycles, stay at Tier 1 (don't jump to Tier 2+). The caller
# tracks dry cycles and passes --dry-cycles N.
#
# Usage:
#   autonomous-waterfall.sh [options]
#
# Options:
#   --control-decision LABEL   Active control label (e.g. autospec:pause).
#                              Empty string / omit = no control signal.
#   --dry-cycles N             Consecutive dry cycles so far (default 0).
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
REPO=""
BACKLOG_COUNT_INJECT=""          # non-empty → skip gh call
DRY_CYCLES_THRESHOLD="${AUTOSPEC_AUTO_DRY_CYCLES:-2}"

# ─── helpers ───────────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
Usage: autonomous-waterfall.sh [options]

Options:
  --control-decision LABEL   Active control label (autospec:pause, etc.).
  --dry-cycles N             Consecutive dry cycles (default 0).
  --repo OWNER/REPO          GitHub repo slug.
  --backlog-count N          Inject backlog count (bypasses gh; for testing).
  -h, --help                 Print this help.

Env:
  AUTOSPEC_AUTO_DRY_CYCLES   Dry-cycle escalation threshold (default 2).

Output (stdout):
  {"tier":<0|1|2|3|4>,"action":"<string>","reason":"<string>"}
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
        --control-decision) CONTROL_DECISION="$2"; shift 2 ;;
        --dry-cycles)       DRY_CYCLES="$2";       shift 2 ;;
        --repo)             REPO="$2";             shift 2 ;;
        --backlog-count)    BACKLOG_COUNT_INJECT="$2"; shift 2 ;;
        -h|--help)          usage; exit 0 ;;
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

# Backlog is empty — check dry-cycle threshold before escalating.
if [ "$DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
    emit 1 "run-backlog" "backlog empty but dry-cycles=$DRY_CYCLES < threshold=$DRY_CYCLES_THRESHOLD; staying at Tier 1"
    exit 0
fi

# ─── Tiers 2–4 — NOT YET ENABLED (Phase 2/3) ──────────────────────────────────
emit 2 "not-yet-enabled" "Tier 2 (local discovery) is not enabled in Phase 1; dry-cycles=$DRY_CYCLES >= threshold=$DRY_CYCLES_THRESHOLD"
exit 0
