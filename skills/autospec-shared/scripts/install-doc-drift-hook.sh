#!/usr/bin/env bash
# install-doc-drift-hook.sh — Optional opt-in pre-commit hook installer.
#
# Usage:
#   install-doc-drift-hook.sh [--target <repo-dir>] [--scripts-dir <path>] [--dry-run]
#
# Appends a marked autospec block to .git/hooks/pre-commit that runs
# check-doc-drift.sh --working-tree on every commit.
#
# The hook prints warnings on drift but NEVER blocks the commit (warnings to
# stderr only; local feedback only — CI is authoritative).
#
# A second install detects the existing autospec block and no-ops (idempotent).
#
# Exit codes:
#   0 = installed or already present
#   1 = error (not a git repo, bad args)

set -euo pipefail

TARGET_DIR=""
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-${HOME}/.autospec/scripts}"
DRY_RUN=0

BLOCK_BEGIN="# BEGIN autospec-doc-drift-hook"
BLOCK_END="# END autospec-doc-drift-hook"

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<USAGE
install-doc-drift-hook.sh — install optional autospec doc-drift pre-commit hook

Usage:
  install-doc-drift-hook.sh [--target <repo-dir>] [--scripts-dir <path>] [--dry-run]

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

# Verify it's a git repo
if [ ! -d "${TARGET_DIR}/.git" ]; then
    err "not a git repository: ${TARGET_DIR}"
    exit 1
fi

HOOK_FILE="${TARGET_DIR}/.git/hooks/pre-commit"

# Idempotency: detect existing block
if [ -f "$HOOK_FILE" ] && grep -qF "$BLOCK_BEGIN" "$HOOK_FILE" 2>/dev/null; then
    info "install-doc-drift-hook: autospec block already present in ${HOOK_FILE} — no-op"
    exit 0
fi

# Build the block to append
HOOK_BLOCK="
${BLOCK_BEGIN}
# autospec doc-drift gate — warnings only, never blocks commit
if command -v bash >/dev/null 2>&1 && [ -f \"${SCRIPTS_DIR}/check-doc-drift.sh\" ]; then
    bash \"${SCRIPTS_DIR}/check-doc-drift.sh\" --working-tree 2>&1 | grep -v '^{' >&2 || true
fi
${BLOCK_END}
"

if [ "$DRY_RUN" -eq 1 ]; then
    if [ -f "$HOOK_FILE" ]; then
        info "[dry-run] install-doc-drift-hook: would append autospec block to ${HOOK_FILE}"
    else
        info "[dry-run] install-doc-drift-hook: would create ${HOOK_FILE} with autospec block"
    fi
    info "[dry-run]   AUTOSPEC_SCRIPTS_DIR → ${SCRIPTS_DIR}"
    exit 0
fi

if [ ! -f "$HOOK_FILE" ]; then
    # Create a minimal pre-commit hook with shebang
    mkdir -p "$(dirname "$HOOK_FILE")"
    printf '#!/usr/bin/env bash\n# pre-commit hook\n' > "$HOOK_FILE"
    chmod +x "$HOOK_FILE"
    info "install-doc-drift-hook: created ${HOOK_FILE}"
fi

# Ensure file is executable
chmod +x "$HOOK_FILE"

# Append the block
printf '%s\n' "$HOOK_BLOCK" >> "$HOOK_FILE"
info "install-doc-drift-hook: appended autospec doc-drift block to ${HOOK_FILE}"
info "  AUTOSPEC_SCRIPTS_DIR → ${SCRIPTS_DIR}"
info "  Note: hook prints warnings only — commits are never blocked"
