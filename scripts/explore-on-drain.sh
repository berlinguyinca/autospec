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
#   3. Dry-well guard: previous explore cycle did not ship 0 PRs
#      (sentinel at ${AUTOSPEC_HOME}/explore-on-drain/<slug>/last-shipped)
#   4. Cycle cap not reached: per-repo counter at
#      ${AUTOSPEC_HOME}/explore-on-drain/<slug>/cycles is strictly less than
#      AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES (default 3)
#
# Per-repo scoping: state lives under a slug-scoped subdirectory so counters
# from different repos never collide (cf. feedback_heartbeat_cross_repo_collision).
# Slug derivation priority:
#   1. AUTOSPEC_REPO env var  (owner/name → owner__name)
#   2. gh repo view           (production path)
#   3. git rev-parse fallback (sanitized path)
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
#   AUTOSPEC_REPO                        override repo slug (owner/name)
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

# ---------------------------------------------------------------------------
# Derive a per-repo slug to scope all state under explore-on-drain/<slug>/.
# Avoids cross-repo counter collisions when the same ~/.autospec is used
# across multiple repositories.
# ---------------------------------------------------------------------------
_derive_slug() {
    local repo=""
    # 1. Explicit env override (used by tests and CI pipelines).
    if [ -n "${AUTOSPEC_REPO:-}" ]; then
        repo="$AUTOSPEC_REPO"
    # 2. gh CLI — production path.
    elif repo="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)" \
         && [ -n "$repo" ]; then
        :
    else
        # 3. Git root path fallback — sanitize to a safe directory name.
        local root
        root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
        printf '%s' "$root" | tr '/' '_' | sed 's/^_//'
        return 0
    fi
    # Canonicalize owner/name → owner__name (matches repo-slug.sh convention).
    printf '%s' "$repo" | sed 's#/#__#'
}

SLUG="$(_derive_slug)"
CYCLES_DIR="${AUTOSPEC_HOME}/explore-on-drain/${SLUG}"
CYCLES_FILE="${CYCLES_DIR}/cycles"
LAST_SHIPPED_FILE="${CYCLES_DIR}/last-shipped"

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

# 3. Dry-well guard — if previous explore cycle shipped zero PRs, stop.
# Avoids infinite chaining when the explore queue is genuinely exhausted.
if [ -f "$LAST_SHIPPED_FILE" ]; then
    _last_shipped="$(cat "$LAST_SHIPPED_FILE")"
    case "$_last_shipped" in
        0)
            echo "stop"
            exit 0
            ;;
        ''|*[!0-9]*)
            : ;;   # non-numeric / empty → treat as unknown; do not block
    esac
fi

# 4. Cycle cap check — read current per-repo counter (default 0).
# Sanitize to numeric: treat missing or non-numeric content as 0 so a
# corrupted counter file does not abort the script under set -eu.
cycles=0
if [ -f "$CYCLES_FILE" ]; then
    raw="$(cat "$CYCLES_FILE")"
    case "$raw" in
        ''|*[!0-9]*) cycles=0 ;;   # empty or non-numeric → reset to 0
        *)            cycles="$raw" ;;
    esac
fi

if [ "$cycles" -ge "$MAX_CYCLES" ]; then
    echo "stop"
    exit 0
fi

# All guardrails passed — increment per-repo counter and emit chain.
mkdir -p "$CYCLES_DIR"
new_cycles=$((cycles + 1))
printf '%s\n' "$new_cycles" > "$CYCLES_FILE"
echo "chain"
exit 0
