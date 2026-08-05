#!/usr/bin/env bash
# install-implementer-precommit.sh — Install autospec pre-commit lint hook into a worktree.
#
# Usage:
#   bash scripts/install-implementer-precommit.sh <worktree-path>
#
# Writes Git's resolved hooks/pre-commit for <worktree-path> and chmods it +x.
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

HOOK_PATH="$(git -C "$WORKTREE" rev-parse --git-path hooks/pre-commit)" || {
    echo "ERROR: failed to resolve Git hooks path for $WORKTREE" >&2
    exit 1
}
case "$HOOK_PATH" in
    /*) ;;
    *) HOOK_PATH="$WORKTREE/$HOOK_PATH" ;;
esac
mkdir -p "$(dirname "$HOOK_PATH")"

cat > "$HOOK_PATH" <<'HOOK_EOF'
#!/usr/bin/env bash
set -euo pipefail
STAGED_BASE_ARGS=()
MERGE_HEAD_PATH=$(git rev-parse --git-path MERGE_HEAD)
if [ -e "$MERGE_HEAD_PATH" ]; then
  if ! MERGE_HEAD=$(git rev-parse --verify 'MERGE_HEAD^{commit}' 2>/dev/null); then
    echo "Pre-commit lint FAILED: MERGE_HEAD is not a valid commit." >&2
    exit 1
  fi
  STAGED_BASE_ARGS=(--staged-base "$MERGE_HEAD")
fi

STAGED=$(git diff --cached --name-only)
[ -z "$STAGED" ] && exit 0

OUT=$(mktemp -t autospec-precommit.XXXXXX)
trap 'rm -f "$OUT"' EXIT

ISSUE_ARGS=()
BRANCH=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
BRANCH_SEGMENT=${BRANCH##*/}
case "$BRANCH" in
  feat/autonomous-issue-[0-9]*)
    ISSUE_NUMBER=${BRANCH_SEGMENT#autonomous-issue-}
    case "$ISSUE_NUMBER" in
      ''|*[!0-9]*) ;;
      *) ISSUE_ARGS=(--issue "$ISSUE_NUMBER") ;;
    esac
    ;;
  */[0-9]*-*)
    ISSUE_NUMBER=${BRANCH_SEGMENT%%-*}
    case "$ISSUE_NUMBER" in
      ''|*[!0-9]*) ;;
      *) ISSUE_ARGS=(--issue "$ISSUE_NUMBER") ;;
    esac
    ;;
esac

# lint-implementation-gates.sh applies the corrected size rules (FILE_GROWTH ratchet,
# and the PR_SIZE changed-line cap waived for a pure shrink) and delegates every other
# rule to lint-implementation.sh unchanged. Falls back to the delegate when the gates
# wrapper is absent, so an older install keeps working.
LINTER="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation-gates.sh"
[ -f "$LINTER" ] || LINTER="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh"

if ! bash "$LINTER" --pre-commit --staged "${STAGED_BASE_ARGS[@]}" "${ISSUE_ARGS[@]}" > "$OUT" 2>&1; then
  echo "Pre-commit lint FAILED. Findings:" >&2
  cat "$OUT" >&2
  echo "" >&2
  echo "Run 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage." >&2
  exit 1
fi
HOOK_EOF

chmod 755 "$HOOK_PATH"

echo "Installed pre-commit hook: $HOOK_PATH"
