#!/usr/bin/env bash
# scripts/explore-on-drain.sh — queue-drain → autospec-explore decision helper.
#
# Emits exactly one of:
#   chain   — all guardrails passed; caller should auto-chain into autospec-explore
#   stop    — default behavior; caller exits normally without chaining
#
# Decision logic (all must hold to emit "chain"):
#   1. Opt-in flag present: ${AUTOSPEC_HOME}/explore-on-drain.flag
#   2. Autonomy gate passes: autospec-autonomy-gate.sh --check all exits 0
#   3. Cycle cap not reached: counter at ${AUTOSPEC_HOME}/explore-on-drain.cycles
#      is strictly less than AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES (default 3)
#
# Side effects:
#   - Increments the cycle counter file ONLY when emitting "chain".
#   - No other side effects; safe to call repeatedly in tests with an
#     isolated HOME / AUTOSPEC_HOME.
#
# Usage:
#   decision=$(bash scripts/explore-on-drain.sh)
#   # decision is "chain" or "stop"
#
# Env:
#   AUTOSPEC_HOME                        default: ~/.autospec
#   AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES default: 3
#
# Bash rules (per project conventions):
#   set -eu; if/then/fi for conditionals (no `[ x ] && y` under set -e);
#   no RETURN traps.
#
# No reuse — autospec-stop-check.sh uses exit-code protocol, not stdout
# chain/stop; this is a different interface per spec.

set -eu

AUTOSPEC_HOME="${AUTOSPEC_HOME:-${HOME}/.autospec}"
MAX_CYCLES="${AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES:-3}"
FLAG_FILE="${AUTOSPEC_HOME}/explore-on-drain.flag"
CYCLES_FILE="${AUTOSPEC_HOME}/explore-on-drain.cycles"

# 1. Flag absent → stop (default unchanged behavior).
if [ ! -f "$FLAG_FILE" ]; then
    echo "stop"
    exit 0
fi

# 2. Autonomy gate check — gate exit non-zero → stop.
if ! autospec-autonomy-gate.sh --check all >/dev/null 2>&1; then
    echo "stop"
    exit 0
fi

# 3. Cycle cap check — read current counter (default 0).
cycles=0
if [ -f "$CYCLES_FILE" ]; then
    cycles="$(cat "$CYCLES_FILE")"
fi

if [ "$cycles" -ge "$MAX_CYCLES" ]; then
    echo "stop"
    exit 0
fi

# All guardrails passed — increment counter and emit chain.
new_cycles=$((cycles + 1))
printf '%s\n' "$new_cycles" > "$CYCLES_FILE"
echo "chain"
exit 0
