#!/usr/bin/env bash
# explore-constitution.sh — seed + enforce the autospec-explore proposal constitution.
#
# Constitutional AI for /autospec-explore: every proposal must satisfy a fixed
# set of rules before it is filed as an auto-implement issue. Deterministic rules
# (D) are enforced here and by the research cycle; judgment rules (J) are applied
# by the TIER_A ranker during the critique-revise step (see the explore SKILL.md
# "Constitution gate" section).
#
# Subcommands:
#   --ensure [--file <path>]        Seed a default constitution if absent (idempotent). Exit 0.
#   --show   [--file <path>]        Print the constitution (seeds first if absent).
#   --filter [--min-confidence N]   Read a JSON array of proposals on stdin, drop those that
#                                   violate the DETERMINISTIC rules, emit the kept array on
#                                   stdout (compact), and print a one-line drop summary to stderr.
#   -h | --help
#
# Deterministic rules enforced by --filter:
#   D1 Evidence  — proposal.evidence must be non-empty.
#   D2 Confidence— proposal.confidence must be >= the floor.
#   D3 Substance — drop bare "chore: address <marker>" (TODO/FIXME/XXX/HACK) churn.
# (Keep these byte-aligned with the cycle's inline check in explore-research-cycle.sh.)
#
# Environment:
#   AUTOSPEC_EXPLORE_CONSTITUTION     constitution path (default: .autospec/explore-constitution.md)
#   AUTOSPEC_EXPLORE_MIN_CONFIDENCE   confidence floor (default: 0.3)
#
# Exit codes:
#   0  ok
#   2  jq missing (fail-closed for the filter) / usage error
#
# Requires: bash 3.2+, jq (for --filter)

set +e

CONSTITUTION="${AUTOSPEC_EXPLORE_CONSTITUTION:-.autospec/explore-constitution.md}"
FLOOR="${AUTOSPEC_EXPLORE_MIN_CONFIDENCE:-0.3}"
MODE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --ensure) MODE=ensure ;;
    --show)   MODE=show ;;
    --filter) MODE=filter ;;
    --file)   shift; CONSTITUTION="${1:-$CONSTITUTION}" ;;
    --min-confidence) shift; FLOOR="${1:-$FLOOR}" ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "explore-constitution: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

_default_constitution() {
  cat <<'MD'
# Explore Proposal Constitution

Rules every `/autospec-explore` proposal must satisfy before it is filed as an
auto-implement issue. Deterministic rules (D) are enforced by
`explore-constitution.sh` and the research cycle; judgment rules (J) are applied
by the TIER_A ranker during the critique-revise step.

## Deterministic (enforced automatically)
- **D1 Evidence** — every proposal MUST cite concrete repo/spec evidence (non-empty `evidence`).
- **D2 Confidence floor** — drop proposals with confidence below `AUTOSPEC_EXPLORE_MIN_CONFIDENCE` (default 0.3).
- **D3 Substance** — drop bare `chore: address <marker>` proposals (raw TODO/FIXME/XXX/HACK churn). These need human triage, not autonomous implementation, and otherwise crowd out substantive spec-drift / prior-report / triaged-issue work.

## Judgment (TIER_A critique-revise before filing)
- **J1 Scope** — prefer ≤ medium complexity; a `large` proposal must be split into a parent + child (`Depends on`) before filing.
- **J2 Testable** — the proposal must imply at least one testable acceptance criterion.
- **J3 Non-duplication** — must not duplicate an open issue or a recently-filed title.
- **J4 Safety** — a proposal touching security-sensitive paths (auth, secrets, crypto, payment) must say so and be flagged for human review.
- **J5 Alignment** — must advance a goal stated in the spec or prior reports, not random churn.

Edit this file to tune the rules for your repo. The deterministic floor is also
read from `AUTOSPEC_EXPLORE_MIN_CONFIDENCE`.
MD
}

_ensure() {
  [ -f "$CONSTITUTION" ] && return 0
  mkdir -p "$(dirname "$CONSTITUTION")" 2>/dev/null
  _default_constitution > "$CONSTITUTION"
  echo "explore-constitution: seeded $CONSTITUTION" >&2
}

case "$MODE" in
  ensure)
    _ensure
    ;;
  show)
    _ensure
    cat "$CONSTITUTION"
    ;;
  filter)
    command -v jq >/dev/null 2>&1 || { echo "explore-constitution: FATAL jq missing" >&2; exit 2; }
    input="$(cat)"
    [ -n "$input" ] || { printf '[]'; exit 0; }
    kept="$(printf '%s' "$input" | jq -c --argjson floor "$FLOOR" \
      '[ .[] | select(((.evidence // "") | tostring | gsub("^\\s+|\\s+$";"")) != "") | select((.confidence // 0) >= $floor) | select(((.title // "") | tostring | test("^\\s*chore:\\s*address\\s+(TODO|FIXME|XXX|HACK)\\b"; "i")) | not) ]' 2>/dev/null)"
    if [ -z "$kept" ]; then
      echo "explore-constitution: WARN filter produced no output (malformed input?)" >&2
      printf '[]'; exit 0
    fi
    n_in="$(printf '%s' "$input" | jq 'length' 2>/dev/null || echo 0)"
    n_out="$(printf '%s' "$kept" | jq 'length' 2>/dev/null || echo 0)"
    echo "explore-constitution: kept=$n_out dropped=$((n_in - n_out)) floor=$FLOOR (D1 evidence, D2 confidence)" >&2
    printf '%s' "$kept"
    ;;
  *)
    echo "explore-constitution: one of --ensure | --show | --filter required" >&2
    exit 2
    ;;
esac
exit 0
