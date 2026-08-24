#!/usr/bin/env bash
# fab-route.sh — Phase 4 label→implementer-gate router (issue #1235).
#
# Decides which Phase 4 implementer gate an issue takes from its labels:
#   * `fab`     — the issue carries `area:fab` or `autospec:fab-flow`; route to
#                 the fab implementer (clean regen → stl-release-gate.py → unittest;
#                 smoke = the model's focused regression test).
#   * `growth`  — the issue carries `growth:artifact`; route to the growth
#                 content-quality gate.
#   * `default` — every other issue keeps the standard implementer + gate.
#
# Precedence: fab > growth > default. Whole-label match only.
#
# A small pure decision helper with a CLI so the bats suite exercises real
# label parsing (no mocks). bash 3.2-safe.
#
# Usage:
#   fab-route.sh --labels "auto-implement,area:fab"   # prints: fab
#   fab-route.sh --labels "auto-implement,growth:artifact"   # prints: growth
#   fab-route.sh --labels "auto-implement,ctx:64k"    # prints: default
#   printf '%s\n' "area:fab" | fab-route.sh --stdin   # prints: fab
#   fab-route.sh -h | --help
#
# Exit codes:
#   0  decision printed (`fab`, `growth`, or `default`)
#   2  usage error (bad args)

set -uo pipefail

# Labels that route an issue to the fab implementer gate.
FAB_LABELS="area:fab autospec:fab-flow"

# Labels that route an issue to the growth content-quality gate.
GROWTH_LABELS="growth:artifact"

usage() {
    printf 'Usage: fab-route.sh --labels "<comma-separated>" | --stdin\n'
}

# route_for_labels <comma-separated labels> -> prints "fab", "growth", or "default".
# Splits on commas, trims surrounding whitespace, and matches each label
# whole (no substring match, so `area:fabric` does NOT route to fab).
# Precedence: fab > growth > default.
route_for_labels() {
    raw="${1:-}"
    saved_ifs="$IFS"
    IFS=','
    # Intentional word-split on comma.
    # shellcheck disable=SC2086
    set -- $raw
    IFS="$saved_ifs"

    # Pass 1: fab wins (highest precedence).
    for label in "$@"; do
        # Trim leading/trailing whitespace (bash 3.2-safe parameter ops).
        label="${label#"${label%%[![:space:]]*}"}"
        label="${label%"${label##*[![:space:]]}"}"
        [ -n "$label" ] || continue
        for fab in $FAB_LABELS; do
            if [ "$label" = "$fab" ]; then
                printf 'fab\n'
                return 0
            fi
        done
    done

    # Pass 2: growth.
    for label in "$@"; do
        # Trim leading/trailing whitespace (bash 3.2-safe parameter ops).
        label="${label#"${label%%[![:space:]]*}"}"
        label="${label%"${label##*[![:space:]]}"}"
        [ -n "$label" ] || continue
        for g in $GROWTH_LABELS; do
            if [ "$label" = "$g" ]; then
                printf 'growth\n'
                return 0
            fi
        done
    done

    printf 'default\n'
    return 0
}

LABELS=""
FROM_STDIN=0

while [ $# -gt 0 ]; do
    case "$1" in
        --labels) LABELS="${2:-}"; shift 2 ;;
        --stdin)  FROM_STDIN=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *)
            printf 'fab-route: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ "$FROM_STDIN" -eq 1 ]; then
    LABELS="$(cat)"
    # Collapse newlines to commas so `gh ... --jq '.labels[].name'` output works.
    LABELS="$(printf '%s' "$LABELS" | tr '\n' ',')"
fi

route_for_labels "$LABELS"
