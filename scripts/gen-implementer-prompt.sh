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
#                                      [--body-file <file>]
#   scripts/gen-implementer-prompt.sh --help
#
# Output (stdout):
#   <static cached prefix from bundle-static-context.sh>
#
#   <dynamic suffix: issue body sections + branch/worktree commands + "begin coding now">
#
# --body-file (autospec:v2-flow cache wiring, spec Phase 2 child C): when given,
# the file's content (e.g. skills/autospec-run/prompts/phase4-implementer.md) is
# handed to bundle-static-context.sh as --static-body, so it is emitted INSIDE the
# cached prefix, ahead of the issue assignment block.
#
# It used to be catted below the closing boundary. The file is one fixed template
# that never varies by issue, so that placement re-sent ~8.9k tokens uncached on
# every dispatch and every retry — 9,241 of the v2 prompt's 16,092 tokens sat
# outside the cache. Only the per-issue suffix belongs below the marker. When
# bundle-static-context.sh is missing, the fallback below still emits the body so
# a degraded install keeps the procedure.
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
BODY_FILE=""
STACK_PROFILE="${PWD}/.autospec/state/stack-profile.json"

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
  --body-file <file>    Prompt body emitted inside the cached prefix, ahead of the
                        issue assignment (e.g. the v2-flow phase4-implementer.md).
  --stack-profile <f>  Read ui_capabilities.gaps from this stack profile and add a
    UI adoption directive when gaps exist, so missing accessible primitives are not
    hand-rolled. Default ./.autospec/state/stack-profile.json; a missing or gap-free
    profile emits nothing.

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
    --body-file)
      [ -z "${2:-}" ] && { printf 'gen-implementer-prompt.sh: --body-file requires a value\n' >&2; exit 1; }
      BODY_FILE="$2"; shift 2 ;;
    --stack-profile)
      [ -z "${2:-}" ] && { printf 'gen-implementer-prompt.sh: --stack-profile requires a value\n' >&2; exit 1; }
      STACK_PROFILE="$2"; shift 2 ;;
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

if [ -n "$BODY_FILE" ] && [ ! -f "$BODY_FILE" ]; then
  printf 'gen-implementer-prompt.sh: body file not found: %s\n' "$BODY_FILE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Emit static cached prefix via bundle-static-context.sh
# ---------------------------------------------------------------------------
# Tracks whether the prefix already carried the body, so the fallback path below
# emits it exactly once.
BODY_IN_PREFIX=0
if [ -f "$BUNDLE_STATIC" ]; then
  # Never swallow this. A silently empty prefix dispatches an implementer with
  # no AGENTS.md and no RULE_ID contract, which looks like a normal run.
  BUNDLE_ARGS=(--role implementer)
  if [ -n "$ISSUE_LABELS" ]; then
    BUNDLE_ARGS+=(--issue-labels "$ISSUE_LABELS")
  fi
  if [ -n "$BODY_FILE" ]; then
    BUNDLE_ARGS+=(--static-body "$BODY_FILE")
    BODY_IN_PREFIX=1
  fi
  bash "$BUNDLE_STATIC" "${BUNDLE_ARGS[@]}"
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
WT_PATH="/tmp/wt-$(echo "$BRANCH" | tr '/' '-' | tr '_' '-')"

printf '\n\n'

# v2-flow cache wiring (spec Phase 2 child C): the body normally rides inside the
# cached prefix via --static-body. This path only runs when
# bundle-static-context.sh was absent, so the body would otherwise be lost.
if [ -n "$BODY_FILE" ] && [ "$BODY_IN_PREFIX" -eq 0 ]; then
  cat "$BODY_FILE"
  printf '\n\n'
fi

# UI capability adoption directive. Hand-rolled widgets are where most
# judgment-class accessibility defects come from, so a repo missing accessible
# primitives is told to adopt them rather than write its own. Silent when the
# profile is absent, the repo is not a web UI, or there are no gaps.
if [ -f "$STACK_PROFILE" ]; then
  # Only the three known capability names are echoed. The profile is a repo-local
  # file, so an arbitrary string from it would be injected into the implementer
  # prompt; allow-listing keeps a crafted stack-profile.json from writing prompt text.
  UI_GAPS="$(python3 -c '
import json, sys
KNOWN = ("accessible_primitives", "motion_library", "reduced_motion_reset")
try:
    caps = json.load(open(sys.argv[1])).get("ui_capabilities") or {}
except Exception:
    caps = {}
gaps = caps.get("gaps") or []
print(" ".join(g for g in KNOWN if g in gaps))
' "$STACK_PROFILE" 2>/dev/null || true)"
  if [ -n "$UI_GAPS" ]; then
    cat <<UIGAPS
### UI capability gaps

This repo is missing: ${UI_GAPS}

Do not hand-roll dialogs, menus, tooltips, comboboxes, tabs, or focus handling.
Adopt an accessible primitive library and a motion library, and add a global
\`prefers-reduced-motion\` reset. If adopting a dependency is out of scope for
this issue, say so and file a follow-up issue instead of writing your own.

UIGAPS
  fi
fi

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
