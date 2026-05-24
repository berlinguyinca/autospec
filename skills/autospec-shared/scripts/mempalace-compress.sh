#!/usr/bin/env bash
# mempalace-compress.sh — Trigger AAAK compression when memory dir exceeds a LOC threshold.
#
# Wraps `mempalace compress` — no-op when below threshold or when mempalace is absent.
#
# Usage:
#   mempalace-compress.sh --dir <path>              # required: memory dir to measure
#   mempalace-compress.sh --dir <path> --threshold <loc>   # default: 5000
#   mempalace-compress.sh --dir <path> --threshold <loc> --dry-run
#   mempalace-compress.sh --dir <path> --threshold <loc> --quiet
#
# Exit codes:
#   0 always (mempalace absence is a silent skip, not a failure)
#
# Environment:
#   AUTOSPEC_COMPRESS_THRESHOLD  — fallback default threshold if --threshold not given
#
# Requires: bash 3.2+, wc
# Optional: mempalace CLI (silent skip when absent)

set +e

DIR=""
THRESHOLD="${AUTOSPEC_COMPRESS_THRESHOLD:-5000}"
DRY_RUN=0
QUIET=0

# ── Argument parse ────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --dir)
      DIR="$2"
      shift 2
      ;;
    --threshold)
      THRESHOLD="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --quiet)
      QUIET=1
      shift
      ;;
    --help|-h)
      printf 'Usage: mempalace-compress.sh --dir <path> [--threshold <loc>] [--dry-run] [--quiet]\n'
      exit 0
      ;;
    *)
      [ "$QUIET" -eq 0 ] && echo "mempalace-compress: unknown argument: $1" >&2
      shift
      ;;
  esac
done

# ── Guard: missing or empty dir → silent no-op ───────────────────────────────
if [ -z "$DIR" ] || [ ! -d "$DIR" ]; then
  exit 0
fi

# ── Guard: mempalace absent → silent skip ────────────────────────────────────
# MEMPALACE_CMD may be overridden for testing; defaults to the system mempalace.
MEMPALACE_CMD="${MEMPALACE_CMD:-mempalace}"
if ! command -v "$MEMPALACE_CMD" > /dev/null 2>&1; then
  exit 0
fi

# ── Measure total LOC in dir ─────────────────────────────────────────────────
total_loc=$(find "$DIR" -maxdepth 3 -name "*.md" -exec wc -l {} + 2>/dev/null \
  | awk '/total$/{print $1}' | tail -1)
total_loc="${total_loc:-0}"

[ "$QUIET" -eq 0 ] && echo "mempalace-compress: dir=$DIR loc=$total_loc threshold=$THRESHOLD"

# ── Below threshold → no-op ──────────────────────────────────────────────────
if [ "$total_loc" -le "$THRESHOLD" ] 2>/dev/null; then
  [ "$QUIET" -eq 0 ] && echo "mempalace-compress: below threshold; no compression needed"
  exit 0
fi

# ── Above threshold → compress ───────────────────────────────────────────────
[ "$QUIET" -eq 0 ] && echo "mempalace-compress: threshold exceeded ($total_loc > $THRESHOLD); compressing"

_compress_args="compress"
if [ "$DRY_RUN" -eq 1 ]; then
  _compress_args="$_compress_args --dry-run"
fi

"$MEMPALACE_CMD" $_compress_args
exit 0
