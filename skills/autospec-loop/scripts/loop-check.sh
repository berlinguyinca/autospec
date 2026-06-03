#!/usr/bin/env bash
# loop-check.sh — run the deterministic CHECK and normalize exit status.
#
# Per docs/specs/2026-06-02-autospec-loop-design.md (§Phase 2):
# Run the CHECK command; map exit 0 -> "met" (exit 0),
# any non-zero -> "not-met" (exit 1). Missing, empty, or literal "null"
# CHECK -> "not-met" (exit 1).
#
# Usage:
#   loop-check.sh [<check-command>]
#
# Output:
#   Prints "met"     to stdout and exits 0 when CHECK succeeds.
#   Prints "not-met" to stdout and exits 1 otherwise.

set -eu

CHECK_CMD="${1:-}"

# Empty or literal "null" means no deterministic check is available.
if [ -z "$CHECK_CMD" ] || [ "$CHECK_CMD" = "null" ]; then
    printf 'not-met\n'
    exit 1
fi

# Run the check command. Capture exit status without aborting under set -e.
check_rc=0
if eval "$CHECK_CMD" > /dev/null 2>&1; then
    check_rc=0
else
    check_rc=$?
fi

if [ "$check_rc" -eq 0 ]; then
    printf 'met\n'
    exit 0
else
    printf 'not-met\n'
    exit 1
fi
