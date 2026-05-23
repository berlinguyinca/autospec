#!/usr/bin/env bash
# packages/cli/scripts/upgrade.sh — re-fetch upstream and re-run install.sh
# Called by: autospec upgrade [--dry-run] [--help]
#
# Resolution order for canonical install.sh (same as install.sh):
#   1. $AUTOSPEC_REPO_ROOT/install.sh
#   2. Relative: ../../../install.sh (packages/cli/scripts/ → repo root)

set -eu

usage() {
  cat <<'EOF'
autospec upgrade — fetch latest autospec and reinstall skills

Usage:
  autospec upgrade [--dry-run] [--help]

Options:
  --dry-run    Print what would be installed without writing files
  --help       Show this help
EOF
}

DRY_RUN=0
PASS_ARGS=()

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    --dry-run) DRY_RUN=1; PASS_ARGS+=(--dry-run) ;;
    *) PASS_ARGS+=("$arg") ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Resolve canonical install.sh
if [ -n "${AUTOSPEC_REPO_ROOT:-}" ] && [ -f "$AUTOSPEC_REPO_ROOT/install.sh" ]; then
  INSTALL_SH="$AUTOSPEC_REPO_ROOT/install.sh"
  REPO_ROOT="$AUTOSPEC_REPO_ROOT"
else
  CANDIDATE_ROOT="$(cd "$SCRIPT_DIR/../../.." 2>/dev/null && pwd)"
  CANDIDATE="$CANDIDATE_ROOT/install.sh"
  if [ -f "$CANDIDATE" ]; then
    INSTALL_SH="$CANDIDATE"
    REPO_ROOT="$CANDIDATE_ROOT"
  else
    echo "autospec upgrade: cannot locate install.sh." >&2
    echo "  Set AUTOSPEC_REPO_ROOT to the autospec repo root." >&2
    exit 1
  fi
fi

# Pull latest from upstream (skip in dry-run)
if [ "$DRY_RUN" -eq 0 ]; then
  if [ -d "$REPO_ROOT/.git" ]; then
    echo "autospec upgrade: pulling latest changes in $REPO_ROOT"
    git -C "$REPO_ROOT" pull --ff-only origin main 2>&1 || {
      echo "autospec upgrade: git pull failed; proceeding with reinstall on current version" >&2
    }
  else
    echo "autospec upgrade: $REPO_ROOT is not a git repo; skipping git pull"
  fi
else
  echo "autospec upgrade: [dry-run] would git pull origin main in $REPO_ROOT"
fi

echo "autospec upgrade: delegating to $INSTALL_SH --update ${PASS_ARGS[*]+"${PASS_ARGS[@]}"}"
exec bash "$INSTALL_SH" --update "${PASS_ARGS[@]+"${PASS_ARGS[@]}"}"
