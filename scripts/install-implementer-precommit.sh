#!/usr/bin/env bash
# install-implementer-precommit.sh — Install autospec pre-commit lint hook into a worktree.
#
# Usage:
#   bash scripts/install-implementer-precommit.sh <worktree-path>
#
# Writes .git/hooks/pre-commit into <worktree-path> and chmods it +x.
# The hook runs lint-implementation.sh --pre-commit --staged and blocks
# commits that contain RULE_ID violations.
#
# Exit codes:
#   0  Hook installed successfully
#   1  Invalid arguments or target not a git worktree

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $(basename "$0") <worktree-path>" >&2
    exit 1
fi

WORKTREE="$1"

if [ ! -d "$WORKTREE" ]; then
    echo "ERROR: worktree path does not exist: $WORKTREE" >&2
    exit 1
fi

if [ ! -d "$WORKTREE/.git" ] && [ ! -f "$WORKTREE/.git" ]; then
    echo "ERROR: $WORKTREE does not appear to be a git repo or worktree (no .git)" >&2
    exit 1
fi

# Resolve the hooks directory (supports both regular repos and worktrees)
if [ -f "$WORKTREE/.git" ]; then
    # Worktree: .git is a file pointing to the gitdir
    GITDIR="$(grep '^gitdir:' "$WORKTREE/.git" | sed 's/^gitdir: //')"
    # Make absolute if relative
    case "$GITDIR" in
        /*) ;;
        *) GITDIR="$WORKTREE/$GITDIR" ;;
    esac
    HOOKS_DIR="$GITDIR/hooks"
else
    HOOKS_DIR="$WORKTREE/.git/hooks"
fi

mkdir -p "$HOOKS_DIR"

HOOK_PATH="$HOOKS_DIR/pre-commit"

cat > "$HOOK_PATH" <<'HOOK_EOF'
#!/usr/bin/env bash
set -euo pipefail
STAGED=$(git diff --cached --name-only)
[ -z "$STAGED" ] && exit 0

OUT=$(mktemp -t autospec-precommit.XXXXXX)
trap 'rm -f "$OUT"' EXIT

if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" --pre-commit --staged > "$OUT" 2>&1; then
  echo "Pre-commit lint FAILED. Findings:" >&2
  cat "$OUT" >&2
  echo "" >&2
  echo "Run 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage." >&2
  exit 1
fi
HOOK_EOF

chmod 755 "$HOOK_PATH"

echo "Installed pre-commit hook: $HOOK_PATH"
