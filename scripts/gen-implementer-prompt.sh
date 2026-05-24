#!/usr/bin/env bash
# scripts/gen-implementer-prompt.sh — compose Phase 4 implementer dispatch prompt.
#
# Wraps bundle-static-context.sh --role implementer (static cached prefix) with a
# deterministic dynamic suffix derived from the issue body and branch name.
# Zero LLM calls.
#
# Usage:
#   scripts/gen-implementer-prompt.sh --issue-body <file> --branch <name>
#                                      [--issue-labels <csv>] [--repo <owner/repo>]
#   scripts/gen-implementer-prompt.sh --help
#
# Output (stdout):
#   <static cached prefix from bundle-static-context.sh>
#
#   <dynamic suffix: issue body sections + branch/worktree commands + "begin coding now">
#
# Dependencies:
#   skills/autospec-shared/scripts/bundle-static-context.sh (or $AUTOSPEC_SCRIPTS_DIR)
#
# Exit codes:
#   0  success
#   1  missing required arg or file not found

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Locate bundle-static-context.sh
BUNDLE_STATIC="${AUTOSPEC_SCRIPTS_DIR:-}"/bundle-static-context.sh
if [ ! -f "$BUNDLE_STATIC" ] 2>/dev/null; then
  # Flat install: bundle-static-context.sh ships as a sibling in the same dir.
  if [ -f "$SCRIPT_DIR/bundle-static-context.sh" ]; then
    BUNDLE_STATIC="$SCRIPT_DIR/bundle-static-context.sh"
  else
    # Fall back to shared scripts in the repo
    BUNDLE_STATIC="$REPO_ROOT/skills/autospec-shared/scripts/bundle-static-context.sh"
  fi
fi

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
ISSUE_BODY_FILE=""
BRANCH=""
ISSUE_LABELS=""
REPO="${AUTOSPEC_REPO:-}"

usage() {
  cat <<'EOF'
gen-implementer-prompt.sh — compose Phase 4 implementer dispatch prompt

Usage:
  scripts/gen-implementer-prompt.sh --issue-body <file> --branch <name>
                                     [--issue-labels <csv>] [--repo <owner/repo>]
  scripts/gen-implementer-prompt.sh --help

Required:
  --issue-body <file>   Path to issue body markdown file
  --branch <name>       Target branch name for the implementation

Optional:
  --issue-labels <csv>  Comma-separated issue labels (for cache-prefix tagging)
  --repo <owner/repo>   Repository slug (default: from AUTOSPEC_REPO env)

Exit: 0=success 1=error
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --issue-body)
      [ -z "${2:-}" ] && { printf 'MISSING_ARG:issue-body\n' >&2; exit 1; }
      ISSUE_BODY_FILE="$2"; shift 2 ;;
    --branch)
      [ -z "${2:-}" ] && { printf 'MISSING_ARG:branch\n' >&2; exit 1; }
      BRANCH="$2"; shift 2 ;;
    --issue-labels)
      [ -z "${2:-}" ] && { printf 'gen-implementer-prompt.sh: --issue-labels requires a value\n' >&2; exit 1; }
      ISSUE_LABELS="$2"; shift 2 ;;
    --repo)
      [ -z "${2:-}" ] && { printf 'gen-implementer-prompt.sh: --repo requires a value\n' >&2; exit 1; }
      REPO="$2"; shift 2 ;;
    *)
      printf 'gen-implementer-prompt.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------
if [ -z "$ISSUE_BODY_FILE" ]; then
  printf 'MISSING_ARG:issue-body\n' >&2
  exit 1
fi

if [ -z "$BRANCH" ]; then
  printf 'MISSING_ARG:branch\n' >&2
  exit 1
fi

if [ ! -f "$ISSUE_BODY_FILE" ]; then
  printf 'gen-implementer-prompt.sh: issue body file not found: %s\n' "$ISSUE_BODY_FILE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Emit static cached prefix via bundle-static-context.sh
# ---------------------------------------------------------------------------
if [ -f "$BUNDLE_STATIC" ]; then
  if [ -n "$ISSUE_LABELS" ]; then
    bash "$BUNDLE_STATIC" --role implementer --issue-labels "$ISSUE_LABELS" 2>/dev/null || true
  else
    bash "$BUNDLE_STATIC" --role implementer 2>/dev/null || true
  fi
else
  # Graceful fallback: emit a minimal cache boundary stub
  printf '<!-- CACHE BOUNDARY -->\n'
  printf '(bundle-static-context.sh not found; static prefix skipped)\n'
  printf '<!-- CACHE BOUNDARY -->\n'
fi

# ---------------------------------------------------------------------------
# Emit dynamic suffix
# ---------------------------------------------------------------------------
ISSUE_BODY="$(cat "$ISSUE_BODY_FILE")"
REPO_SLUG="${REPO:-berlinguyinca/autospec}"
WT_PATH="/private/tmp/wt-$(echo "$BRANCH" | tr '/' '-' | tr '_' '-')"

printf '\n\n'
cat <<SUFFIX
---

## Your implementation assignment

**Branch:** \`${BRANCH}\`
**Repository:** \`${REPO_SLUG}\`

### Worktree setup

\`\`\`bash
cd /path/to/autospec
git fetch origin
git worktree add ${WT_PATH} -b ${BRANCH} origin/main
cd ${WT_PATH}
\`\`\`

### Issue body

${ISSUE_BODY}

---

begin coding now
SUFFIX
