#!/usr/bin/env bash
# scripts/autospec-stop-check.sh — single-point stop-sentinel check for the
# autospec inner loop (inside process(ISSUE), after each major step).
#
# Usage:
#   autospec-stop-check.sh <ISSUE> <BRANCH> <LAST_STEP>
#   autospec-stop-check.sh --help
#
# Exit codes:
#   0   — no immediate stop flag; caller may continue
#   42  — immediate stop flag found; caller MUST abort current issue and exit 0
#         (see contract below)
#
# Caller contract:
#   if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" \
#       "$ISSUE" "$BRANCH" "$LAST_STEP"; then
#     exit 0
#   fi
#
# The exit-42 / "if ! ..." pattern means: "if the check says abort, stop the
# current issue handler cleanly (exit 0 from the caller's perspective)."
#
# Sentinel file: ~/.autospec/stop.flag
#   Line 1: graceful | immediate
#   (Only "immediate" triggers an abort at the per-issue level; "graceful"
#    is honoured by the outer-loop sentinel, which is NOT replaced by this
#    script — the outer-loop check remains inline because it also writes the
#    batch-done.json summary.)
#
# Bash rules (per spec): no RETURN traps; if/then/fi for conditionals (not
# `[ x ] && y` under set -e).
#
# Dependencies: bash, head (POSIX), the autospec-stop.sh helper.

set -eu

# ---------------------------------------------------------------------------
# --help
# ---------------------------------------------------------------------------
if [ $# -ge 1 ] && [ "$1" = "--help" ]; then
  cat <<'HELP'
autospec-stop-check.sh — per-issue stop-sentinel check

Usage:
  autospec-stop-check.sh <ISSUE> <BRANCH> <LAST_STEP>
  autospec-stop-check.sh --help

Exit codes:
  0   no immediate stop flag; caller continues
  42  immediate stop flag found; caller should abort and exit 0

Caller contract (copy-paste):
  if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" \
      "$ISSUE" "$BRANCH" "$LAST_STEP"; then
    exit 0
  fi

The flag file is ~/.autospec/stop.flag.  Only "immediate" triggers a
per-issue abort; "graceful" is handled by the outer-loop sentinel.
HELP
  exit 0
fi

# ---------------------------------------------------------------------------
# Argument validation
# ---------------------------------------------------------------------------
if [ $# -ne 3 ]; then
  printf 'autospec-stop-check.sh: error: expected 3 args (ISSUE BRANCH LAST_STEP), got %d\n' "$#" >&2
  printf 'Run with --help for usage.\n' >&2
  exit 2
fi

ISSUE="$1"
BRANCH="$2"
LAST_STEP="$3"

FLAG_FILE="${HOME}/.autospec/stop.flag"

# ---------------------------------------------------------------------------
# Check: only act on "immediate" stop
# ---------------------------------------------------------------------------
if [ ! -f "$FLAG_FILE" ]; then
  exit 0
fi

MODE="$(head -1 "$FLAG_FILE" 2>/dev/null || echo "")"

if [ "$MODE" = "immediate" ]; then
  STOP_SH="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh"
  if [ -x "$STOP_SH" ]; then
    bash "$STOP_SH" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP" || true
  else
    printf '[autospec-stop-check] WARN: autospec-stop.sh not found at %s; flag acknowledged\n' \
      "$STOP_SH" >&2
  fi
  exit 42
fi

exit 0
