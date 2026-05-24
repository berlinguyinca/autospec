#!/usr/bin/env bash
# scripts/gen-reviewer-prompt.sh — compose Phase 4 fused guardian+LGTM reviewer prompt.
#
# Wraps bundle-static-context.sh --role reviewer (static cached prefix) with a
# deterministic dynamic suffix derived from the PR diff and previous-iteration findings.
# Zero LLM calls.
#
# Usage:
#   scripts/gen-reviewer-prompt.sh --pr-diff <file> [--prev-findings <file>]
#                                   [--issue-labels <csv>] [--repo <owner/repo>]
#   scripts/gen-reviewer-prompt.sh --help
#
# Output (stdout):
#   <static cached prefix from bundle-static-context.sh --role reviewer>
#
#   <dynamic suffix: clipped PR diff + prev findings + fused guardian+LGTM rubric>
#
# Diff clipping: input over 8000 lines is clipped with a "[diff clipped at 8000 lines]" marker.
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
PR_DIFF_FILE=""
PREV_FINDINGS_FILE=""
ISSUE_LABELS=""
REPO="${AUTOSPEC_REPO:-}"

DIFF_CLIP_LINES=8000

usage() {
  cat <<'EOF'
gen-reviewer-prompt.sh — compose Phase 4 fused guardian+LGTM reviewer prompt

Usage:
  scripts/gen-reviewer-prompt.sh --pr-diff <file> [--prev-findings <file>]
                                  [--issue-labels <csv>] [--repo <owner/repo>]
  scripts/gen-reviewer-prompt.sh --help

Required:
  --pr-diff <file>         Path to PR diff file

Optional:
  --prev-findings <file>   Path to previous-iteration findings file (JSON or text)
  --issue-labels <csv>     Comma-separated issue labels (for cache-prefix tagging)
  --repo <owner/repo>      Repository slug (default: from AUTOSPEC_REPO env)

Diff clipping: diffs over 8000 lines are clipped with a "[diff clipped at 8000 lines]" marker.

Exit: 0=success 1=error
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --pr-diff)
      [ -z "${2:-}" ] && { printf 'MISSING_ARG:pr-diff\n' >&2; exit 1; }
      PR_DIFF_FILE="$2"; shift 2 ;;
    --prev-findings)
      [ -z "${2:-}" ] && { printf 'gen-reviewer-prompt.sh: --prev-findings requires a value\n' >&2; exit 1; }
      PREV_FINDINGS_FILE="$2"; shift 2 ;;
    --issue-labels)
      [ -z "${2:-}" ] && { printf 'gen-reviewer-prompt.sh: --issue-labels requires a value\n' >&2; exit 1; }
      ISSUE_LABELS="$2"; shift 2 ;;
    --repo)
      [ -z "${2:-}" ] && { printf 'gen-reviewer-prompt.sh: --repo requires a value\n' >&2; exit 1; }
      REPO="$2"; shift 2 ;;
    *)
      printf 'gen-reviewer-prompt.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------
if [ -z "$PR_DIFF_FILE" ]; then
  printf 'MISSING_ARG:pr-diff\n' >&2
  exit 1
fi

if [ ! -f "$PR_DIFF_FILE" ]; then
  printf 'gen-reviewer-prompt.sh: pr-diff file not found: %s\n' "$PR_DIFF_FILE" >&2
  exit 1
fi

if [ -n "$PREV_FINDINGS_FILE" ] && [ ! -f "$PREV_FINDINGS_FILE" ]; then
  printf 'gen-reviewer-prompt.sh: prev-findings file not found: %s\n' "$PREV_FINDINGS_FILE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Emit static cached prefix via bundle-static-context.sh --role reviewer
# ---------------------------------------------------------------------------
if [ -f "$BUNDLE_STATIC" ]; then
  if [ -n "$ISSUE_LABELS" ]; then
    bash "$BUNDLE_STATIC" --role reviewer --issue-labels "$ISSUE_LABELS" 2>/dev/null || true
  else
    bash "$BUNDLE_STATIC" --role reviewer 2>/dev/null || true
  fi
else
  # Graceful fallback: emit a minimal cache boundary stub
  printf '<!-- CACHE BOUNDARY -->\n'
  printf '(bundle-static-context.sh not found; static prefix skipped)\n'
  printf '<!-- CACHE BOUNDARY -->\n'
fi

# ---------------------------------------------------------------------------
# Clip diff to DIFF_CLIP_LINES
# ---------------------------------------------------------------------------
diff_line_count=$(wc -l < "$PR_DIFF_FILE" || echo 0)
diff_clipped=0
if [ "$diff_line_count" -gt "$DIFF_CLIP_LINES" ]; then
  DIFF_CONTENT=$(head -n "$DIFF_CLIP_LINES" "$PR_DIFF_FILE")
  diff_clipped=1
else
  DIFF_CONTENT=$(cat "$PR_DIFF_FILE")
fi

# ---------------------------------------------------------------------------
# Read previous-iteration findings (empty if not provided)
# ---------------------------------------------------------------------------
PREV_FINDINGS=""
if [ -n "$PREV_FINDINGS_FILE" ]; then
  PREV_FINDINGS=$(cat "$PREV_FINDINGS_FILE")
fi

# ---------------------------------------------------------------------------
# Emit dynamic suffix
# ---------------------------------------------------------------------------
REPO_SLUG="${REPO:-berlinguyinca/autospec}"

printf '\n\n'
cat <<SUFFIX
---

## Your review assignment

**Repository:** \`${REPO_SLUG}\`

### PR diff

\`\`\`diff
${DIFF_CONTENT}
SUFFIX

if [ "$diff_clipped" -eq 1 ]; then
  printf '[diff clipped at 8000 lines]\n'
fi

cat <<SUFFIX2
\`\`\`

### Previous-iteration findings

\`\`\`
${PREV_FINDINGS}
\`\`\`

---

## Fused guardian+LGTM review rubric

**Part 1 — Guardian (contract compliance)** — skip if \`AUTOSPEC_NO_GUARDIAN=1\`:
1. Read AGENTS.md \`## Implementation-quality contract\` for the RULE_ID table and directive map.
2. Read the issue body — note \`## Implementation scope\`, \`## Implementation outline\`, \`## Tests required\`, and any \`Guardian: skip-*\` lines.
3. Read deterministic findings from lint-implementation.sh output (included above if present).
4. Apply LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). Collect as \`RULE_ID:<path>:<line>: <desc>\`. Honor \`Guardian: skip-*\` with \`INFO:\` lines.

**Part 2 — LGTM (correctness review):**
5. Check correctness, edge cases, missing tests, AGENTS.md compliance (TDD, no mocks, conventional commits).
6. Collect findings as a numbered list.

**Hard limit:** max 25 tool calls total (Parts 1 + 2 combined). If budget exhausted, append \`RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review\` and proceed to verdict.

**Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: \`LGTM\`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings.
SUFFIX2
