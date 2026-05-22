#!/usr/bin/env bash
# install-doc-drift-workflow.sh — Idempotent installer for the autospec doc-drift CI workflow.
#
# Usage:
#   install-doc-drift-workflow.sh [--target <repo-dir>] [--scripts-dir <path>] [--dry-run]
#
# Copies .github/workflows/autospec-doc-drift.yml into the target repo.
# If the file already exists and is identical → no-op (idempotent).
# If the file already exists and differs → prints a non-blocking warning and keeps existing.
#
# The installer rewrites AUTOSPEC_SCRIPTS_PATH in the workflow to point at
# the consumer's $AUTOSPEC_SCRIPTS_DIR (defaults to ~/.autospec/scripts).
#
# Exit codes:
#   0 = installed or already up-to-date
#   1 = error (missing template, bad args)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Template lives next to this installer or in the autospec repo root
TEMPLATE_SEARCH_PATHS=(
    "${SCRIPT_DIR}/../../../.github/workflows/autospec-doc-drift.yml"
    "${SCRIPT_DIR}/../../../../.github/workflows/autospec-doc-drift.yml"
)

TARGET_DIR=""
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-${HOME}/.autospec/scripts}"
DRY_RUN=0

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<USAGE
install-doc-drift-workflow.sh — install autospec doc-drift CI workflow

Usage:
  install-doc-drift-workflow.sh [--target <repo-dir>] [--scripts-dir <path>] [--dry-run]

Options:
  --target <dir>      Target repository root (default: current directory)
  --scripts-dir <p>   Path autospec scripts are installed at in the target
                      (default: \$AUTOSPEC_SCRIPTS_DIR or ~/.autospec/scripts)
  --dry-run           Print actions, write nothing
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            shift
            TARGET_DIR="${1:-}"
            ;;
        --target=*)
            TARGET_DIR="${1#--target=}"
            ;;
        --scripts-dir)
            shift
            SCRIPTS_DIR="${1:-}"
            ;;
        --scripts-dir=*)
            SCRIPTS_DIR="${1#--scripts-dir=}"
            ;;
        --dry-run)
            DRY_RUN=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "unknown argument: $1"
            usage >&2
            exit 1
            ;;
    esac
    shift
done

# Default target: current directory
if [ -z "$TARGET_DIR" ]; then
    TARGET_DIR="$(pwd)"
fi

if [ ! -d "$TARGET_DIR" ]; then
    err "target directory does not exist: $TARGET_DIR"
    exit 1
fi

# Find template
TEMPLATE=""
for candidate in "${TEMPLATE_SEARCH_PATHS[@]}"; do
    if [ -f "$candidate" ]; then
        TEMPLATE="$(cd "$(dirname "$candidate")" && pwd)/$(basename "$candidate")"
        break
    fi
done

if [ -z "$TEMPLATE" ]; then
    err "cannot find autospec-doc-drift.yml template"
    err "searched: ${TEMPLATE_SEARCH_PATHS[*]}"
    exit 1
fi

DEST_DIR="${TARGET_DIR}/.github/workflows"
DEST="${DEST_DIR}/autospec-doc-drift.yml"

# Rewrite AUTOSPEC_SCRIPTS_PATH line in template content
CONTENT="$(sed "s|AUTOSPEC_SCRIPTS_PATH=\"\${HOME}/\.autospec/scripts\"|AUTOSPEC_SCRIPTS_PATH=\"${SCRIPTS_DIR}\"|g" "$TEMPLATE")"

if [ -f "$DEST" ]; then
    EXISTING="$(cat "$DEST")"
    if [ "$EXISTING" = "$CONTENT" ]; then
        info "install-doc-drift-workflow: already up-to-date at ${DEST}"
        exit 0
    else
        warn "install-doc-drift-workflow: ${DEST} exists but differs from template"
        warn "  Keeping existing file. To upgrade, delete it and re-run."
        exit 0
    fi
fi

if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] install-doc-drift-workflow: would create ${DEST}"
    info "[dry-run]   AUTOSPEC_SCRIPTS_PATH → ${SCRIPTS_DIR}"
    exit 0
fi

mkdir -p "$DEST_DIR"
printf '%s\n' "$CONTENT" > "$DEST"
info "install-doc-drift-workflow: installed ${DEST}"
info "  AUTOSPEC_SCRIPTS_PATH → ${SCRIPTS_DIR}"
