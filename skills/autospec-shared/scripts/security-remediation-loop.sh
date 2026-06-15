#!/usr/bin/env bash
# security-remediation-loop.sh — run security-scan.sh and decide block vs pass.
#
# The caller drives the actual code fix between rounds; this script owns the
# scan invocation, the block/pass decision, the round cap (informational), and
# the secret rotation annotation. One decision per --decide invocation; the
# caller loops: fix -> re-invoke until pass or AUTOSPEC_SEC_MAX_ROUNDS.
#
# Usage:
#   security-remediation-loop.sh --decide [--diff <base>] [--root <dir>]
#
# Block rule: decision=block (exit 1) iff any finding has severity==must-fix.
#             Otherwise decision=pass (exit 0). nice-to-have never blocks.
# Secret rule: every surviving must-fix secrets gap also prints a
#             "ROTATE: <file> — <title>" line to stdout for the PR body.
#
# Environment:
#   AUTOSPEC_SECSCAN_BIN    — path to security-scan.sh (default: sibling)
#   AUTOSPEC_SEC_MAX_ROUNDS — informational cap echoed for the caller (default 3)
#   AUTOSPEC_STATE_DIR      — state dir (default: ~/.autospec)
#
# Exit codes:
#   0  decision=pass
#   1  decision=block (must-fix survivors)
#   2  scan engine failed closed (could not run)
#
# Requires: bash 3.2+, jq

set +e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCAN_BIN="${AUTOSPEC_SECSCAN_BIN:-$SCRIPT_DIR/security-scan.sh}"
MAX_ROUNDS="${AUTOSPEC_SEC_MAX_ROUNDS:-3}"

DIFF="" ROOT="." DECIDE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --decide) DECIDE=1 ;;
    --diff) shift; DIFF="${1:-}" ;;
    --root) shift; ROOT="${1:-.}" ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "remediation-loop: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

# Build scan args.
set --
if [ -n "$DIFF" ]; then set -- --diff "$DIFF"; else set -- --tree; fi
set -- "$@" --root "$ROOT"

findings="$("$SCAN_BIN" "$@" 2>/dev/null)"; rc=$?
if [ "$rc" -eq 2 ]; then
  echo "decision=block reason=engine-failed-closed"
  exit 2
fi

mustfix="$(printf '%s\n' "$findings" | jq -rc 'select(.severity=="must-fix")' 2>/dev/null)"

# Secret rotation annotations (one ROTATE line per surviving must-fix secret).
printf '%s\n' "$mustfix" | jq -r 'select(.dimension=="secrets") | "ROTATE: \(.file) — \(.title)"' 2>/dev/null

# Count must-fix objects (lines that look like JSON objects).
count="$(printf '%s\n' "$mustfix" | grep -c '^{')"
if [ "$count" -gt 0 ]; then
  echo "decision=block must_fix=$count max_rounds=$MAX_ROUNDS"
  exit 1
fi
echo "decision=pass max_rounds=$MAX_ROUNDS"
exit 0
