#!/usr/bin/env bash
# skills/autospec-harmonize/scripts/fetch-vendor.sh
#
# Wrapper for fetching a vendor palette from the autospec-design catalog.
# Usage: fetch-vendor.sh --vendor <name> --out <file>
#
# On success: writes JSON to <file>, exits 0.
# On failure: prints code_health:harmonize_vendor_fetch_failed to stderr, exits 1.

set -euo pipefail

VENDOR_NAME=""
OUT_FILE=""

# Parse args
i=0
args=("$@")
while [ $i -lt ${#args[@]} ]; do
  if [ "${args[$i]}" = "--vendor" ]; then
    i=$((i + 1))
    VENDOR_NAME="${args[$i]}"
  elif [ "${args[$i]}" = "--out" ]; then
    i=$((i + 1))
    OUT_FILE="${args[$i]}"
  fi
  i=$((i + 1))
done

if [ -z "$VENDOR_NAME" ] || [ -z "$OUT_FILE" ]; then
  printf 'code_health:harmonize_vendor_fetch_failed\n' >&2
  exit 1
fi

# Locate autospec-design fetch script (if present in known relative paths)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
DESIGN_FETCH="$REPO_ROOT/skills/autospec-design/scripts/fetch-catalog.sh"

if [ -f "$DESIGN_FETCH" ]; then
  if "$DESIGN_FETCH" --vendor "$VENDOR_NAME" --out "$OUT_FILE"; then
    exit 0
  else
    printf 'code_health:harmonize_vendor_fetch_failed\n' >&2
    exit 1
  fi
fi

# Fallback: attempt curl to a catalog URL (environment-configurable)
CATALOG_URL="${AUTOSPEC_VENDOR_CATALOG_URL:-}"
if [ -n "$CATALOG_URL" ]; then
  if curl -fsSL "${CATALOG_URL}/${VENDOR_NAME}.json" -o "$OUT_FILE" 2>/dev/null; then
    exit 0
  else
    printf 'code_health:harmonize_vendor_fetch_failed\n' >&2
    exit 1
  fi
fi

# No fetch path available
printf 'code_health:harmonize_vendor_fetch_failed\n' >&2
exit 1
