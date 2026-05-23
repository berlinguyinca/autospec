#!/usr/bin/env bash
# packages/cli/scripts/install.sh — wrapper over canonical install.sh
# Called by: autospec install [--dry-run] [--skill <name>] [--harness <name>] [--update]
#
# Locates the autospec repo's root install.sh and delegates to it.
# Resolution order:
#   1. $AUTOSPEC_REPO_ROOT/install.sh      — explicit override
#   2. Relative to this script: ../../../../install.sh  (packages/cli/scripts/ → repo root)
#   3. Exit 1 with helpful error message

set -eu

usage() {
  cat <<'EOF'
autospec install — install autospec skills and scripts into your harness

Delegates to the canonical install.sh in the autospec repo root.

Usage:
  autospec install [options]

Options:
  --dry-run         Print actions without writing files
  --skill <name>    Install only the named skill (default: all)
  --harness <name>  Install only for the named harness (default: all)
  --update          Idempotent re-install (overwrite existing)
  --help            Show this help
EOF
}

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Resolution order for canonical install.sh
if [ -n "${AUTOSPEC_REPO_ROOT:-}" ] && [ -f "$AUTOSPEC_REPO_ROOT/install.sh" ]; then
  INSTALL_SH="$AUTOSPEC_REPO_ROOT/install.sh"
else
  # Relative from packages/cli/scripts/ → ../../../ = repo root
  CANDIDATE="$(cd "$SCRIPT_DIR/../../.." 2>/dev/null && pwd)/install.sh"
  if [ -f "$CANDIDATE" ]; then
    INSTALL_SH="$CANDIDATE"
  else
    echo "autospec install: cannot locate install.sh." >&2
    echo "  Set AUTOSPEC_REPO_ROOT to the autospec repo root, or install via:" >&2
    echo "    bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/install.sh)" >&2
    exit 1
  fi
fi

echo "autospec install: delegating to $INSTALL_SH"
exec bash "$INSTALL_SH" "$@"
