#!/usr/bin/env bash
# run-batch-start.sh — record / read the UTC run-start timestamp for an autospec run.
#
# autospec-run writes this once at run-start so the end-of-run gap-remediation
# phase can scope `/autospec-review --since` to work shipped during THIS run.
# Format: ISO 8601 UTC, e.g. 2026-05-24T18:42:11Z (matches `date -u +%Y-%m-%dT%H:%M:%SZ`).
#
# Usage:
#   run-batch-start.sh --write [--force]   # write now() unless file exists (--force overwrites)
#   run-batch-start.sh --read              # echo stored timestamp (epoch sentinel if absent)
#   run-batch-start.sh --help
#
# Environment:
#   AUTOSPEC_STATE_DIR  — override state directory (default: ~/.autospec)
#
# Exit codes:
#   0  always (best-effort; missing file on --read returns epoch sentinel)
#
# Requires: bash 3.2+, date

set +e

STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
BATCH_FILE="$STATE_DIR/.run-batch-start"
EPOCH_SENTINEL="1970-01-01T00:00:00Z"

MODE=""
FORCE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --write) MODE="write"; shift ;;
    --read)  MODE="read";  shift ;;
    --force) FORCE=1;      shift ;;
    --help|-h)
      printf 'Usage: run-batch-start.sh --write [--force] | --read\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

case "$MODE" in
  write)
    mkdir -p "$STATE_DIR" 2>/dev/null
    if [ -f "$BATCH_FILE" ] && [ "$FORCE" -ne 1 ]; then
      exit 0
    fi
    date -u +%Y-%m-%dT%H:%M:%SZ > "$BATCH_FILE" 2>/dev/null
    exit 0
    ;;
  read)
    if [ -f "$BATCH_FILE" ]; then
      cat "$BATCH_FILE"
    else
      printf '%s\n' "$EPOCH_SENTINEL"
    fi
    exit 0
    ;;
  *)
    printf 'Usage: run-batch-start.sh --write [--force] | --read\n' >&2
    exit 0
    ;;
esac
