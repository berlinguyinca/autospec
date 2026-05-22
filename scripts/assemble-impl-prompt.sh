#!/usr/bin/env bash
# scripts/assemble-impl-prompt.sh — Assemble enriched implementer prompt sections.
#
# Usage:
#   assemble-impl-prompt.sh <issue-number>
#     Fetch issue from GitHub, select matching memory files, emit prompt sections.
#
#   assemble-impl-prompt.sh --issue-labels <comma-list> \
#       --memory-dir <dir> --manifest <yml> --agents-md <file> --ac-body <file>
#     Low-level mode for testing (no GitHub API call).
#
# Output (stdout):
#   ## Project rules you MUST honor
#   <verbatim concatenation of matching feedback_*.md bodies>
#
#   ## RULE_IDs (from AGENTS.md ## Implementation-quality contract)
#   <RULE_ID table from AGENTS.md>
#
#   ## Acceptance criteria as constraints
#   <AC checkbox list from issue body>
#
# Exit codes:
#   0  Success
#   1  Argument/file error
#
# Compatible with bash 3.2+

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

HELP_TEXT='Usage:
  assemble-impl-prompt.sh <issue-number>
  assemble-impl-prompt.sh --issue-labels <labels> --memory-dir <dir> \
      --manifest <yml> --agents-md <file> --ac-body <file>
  assemble-impl-prompt.sh --help

Assemble enriched Phase 4 implementer prompt sections from saved memory,
RULE_IDs, and acceptance criteria.'

ISSUE_NUMBER=""
ISSUE_LABELS=""
MEMORY_DIR=""
MANIFEST=""
AGENTS_MD=""
AC_BODY=""
LOW_LEVEL=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      printf '%s\n' "$HELP_TEXT"
      exit 0
      ;;
    --issue-labels)
      if [ $# -lt 2 ]; then printf 'assemble-impl-prompt.sh: --issue-labels requires an argument\n' >&2; exit 1; fi
      ISSUE_LABELS="$2"
      LOW_LEVEL=1
      shift 2
      ;;
    --memory-dir)
      if [ $# -lt 2 ]; then printf 'assemble-impl-prompt.sh: --memory-dir requires an argument\n' >&2; exit 1; fi
      MEMORY_DIR="$2"
      shift 2
      ;;
    --manifest)
      if [ $# -lt 2 ]; then printf 'assemble-impl-prompt.sh: --manifest requires an argument\n' >&2; exit 1; fi
      MANIFEST="$2"
      shift 2
      ;;
    --agents-md)
      if [ $# -lt 2 ]; then printf 'assemble-impl-prompt.sh: --agents-md requires an argument\n' >&2; exit 1; fi
      AGENTS_MD="$2"
      shift 2
      ;;
    --ac-body)
      if [ $# -lt 2 ]; then printf 'assemble-impl-prompt.sh: --ac-body requires an argument\n' >&2; exit 1; fi
      AC_BODY="$2"
      shift 2
      ;;
    -*)
      printf 'assemble-impl-prompt.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      if [ -z "$ISSUE_NUMBER" ]; then
        ISSUE_NUMBER="$1"
      else
        printf 'assemble-impl-prompt.sh: unexpected argument: %s\n' "$1" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

# ── resolve inputs ────────────────────────────────────────────────────────────

if [ "$LOW_LEVEL" -eq 0 ]; then
  if [ -z "$ISSUE_NUMBER" ]; then
    printf '%s\n' "$HELP_TEXT" >&2
    exit 1
  fi

  REPO=$(git -C "$REPO_ROOT" remote get-url origin 2>/dev/null \
    | sed 's|.*github\.com[:/]\(.*\)\.git|\1|; s|.*github\.com[:/]\(.*\)|\1|' || echo "")
  if [ -z "$REPO" ]; then
    printf 'assemble-impl-prompt.sh: could not detect GitHub repo from git remote\n' >&2
    exit 1
  fi

  ISSUE_JSON=$(gh issue view "$ISSUE_NUMBER" --repo "$REPO" --json labels,body 2>/dev/null)
  ISSUE_LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null || echo "")
  ISSUE_BODY_TMP=$(mktemp /tmp/assemble-ac-XXXXXX.md)
  printf '%s' "$ISSUE_JSON" | jq -r '.body' > "$ISSUE_BODY_TMP"
  AC_BODY="$ISSUE_BODY_TMP"

  WHOAMI=$(whoami 2>/dev/null || echo "$(id -un)")
  MEMORY_DIR="${AUTOSPEC_MEMORY_DIR:-$HOME/.claude/projects/-Users-${WHOAMI}-IdeaProjects-autospec/memory}"
  MANIFEST="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/memory-tags.yml"
  AGENTS_MD="$REPO_ROOT/AGENTS.md"
fi

# Validate required inputs
if [ -z "$MEMORY_DIR" ]; then printf 'assemble-impl-prompt.sh: --memory-dir is required\n' >&2; exit 1; fi
if [ -z "$MANIFEST" ]; then printf 'assemble-impl-prompt.sh: --manifest is required\n' >&2; exit 1; fi
if [ -z "$AGENTS_MD" ]; then printf 'assemble-impl-prompt.sh: --agents-md is required\n' >&2; exit 1; fi
if [ -z "$AC_BODY" ]; then printf 'assemble-impl-prompt.sh: --ac-body is required\n' >&2; exit 1; fi

if [ ! -d "$MEMORY_DIR" ]; then printf 'assemble-impl-prompt.sh: memory-dir not found: %s\n' "$MEMORY_DIR" >&2; exit 1; fi
if [ ! -f "$MANIFEST" ]; then printf 'assemble-impl-prompt.sh: manifest not found: %s\n' "$MANIFEST" >&2; exit 1; fi
if [ ! -f "$AGENTS_MD" ]; then printf 'assemble-impl-prompt.sh: agents-md not found: %s\n' "$AGENTS_MD" >&2; exit 1; fi
if [ ! -f "$AC_BODY" ]; then printf 'assemble-impl-prompt.sh: ac-body not found: %s\n' "$AC_BODY" >&2; exit 1; fi

# ── tag intersection ──────────────────────────────────────────────────────────

# Normalize issue labels to space-separated lowercase tokens
issue_tokens=" $(printf '%s' "$ISSUE_LABELS" | tr ',' ' ' | tr '[:upper:]' '[:lower:]') "

# Build list of matched memory file paths by reading manifest line by line
matched_files_tmp=$(mktemp /tmp/assemble-matched-XXXXXX.txt)
trap 'rm -f "$matched_files_tmp" 2>/dev/null; [ -n "${ISSUE_BODY_TMP:-}" ] && rm -f "$ISSUE_BODY_TMP" 2>/dev/null' EXIT

current_file=""
while IFS= read -r line; do
  # Detect filename line: feedback_foo.md:
  if printf '%s' "$line" | grep -qE '^feedback_[^:]+\.md:'; then
    current_file=$(printf '%s' "$line" | sed 's/:.*//')
    continue
  fi
  # Detect tags line:   tags: [a, b, c]
  if printf '%s' "$line" | grep -qE '^[[:space:]]+tags:' && [ -n "$current_file" ]; then
    tags_raw=$(printf '%s' "$line" | sed 's/.*\[//; s/\].*//')
    current_path="$MEMORY_DIR/$current_file"
    match=0
    # Check each tag against issue_tokens
    for tag in $(printf '%s' "$tags_raw" | tr ',' '\n' | tr -d ' []"'"'" | tr '[:upper:]' '[:lower:]'); do
      if printf '%s' "$issue_tokens" | grep -qw "$tag"; then
        match=1
        break
      fi
    done
    if [ "$match" -eq 1 ] && [ -f "$current_path" ]; then
      printf '%s\n' "$current_path" >> "$matched_files_tmp"
    fi
    current_file=""
  fi
done < "$MANIFEST"

# ── emit prompt sections ──────────────────────────────────────────────────────

printf '## Project rules you MUST honor\n\n'

matched_count=$(wc -l < "$matched_files_tmp" | tr -d ' ')
if [ "$matched_count" -eq 0 ]; then
  printf '(No memory files matched this issue'"'"'s labels.)\n\n'
else
  while IFS= read -r mem_file; do
    [ -f "$mem_file" ] || continue
    # Strip YAML frontmatter (---...---) if present
    awk '
      BEGIN { in_front=0; done_front=0 }
      /^---$/ && !done_front && NR==1 { in_front=1; next }
      /^---$/ && in_front { in_front=0; done_front=1; next }
      !in_front { print }
    ' "$mem_file"
    printf '\n'
  done < "$matched_files_tmp"
fi

printf '## RULE_IDs (from AGENTS.md ## Implementation-quality contract)\n\n'

awk '
  /^### RULE_ID table/ { in_table=1; next }
  /^### / && in_table { exit }
  /^## / && in_table { exit }
  in_table { print }
' "$AGENTS_MD"

printf '\n## Acceptance criteria as constraints\n\n'
printf '(Every checkbox below must be green before you push.)\n\n'

awk '
  /^## Acceptance criteria/ { in_ac=1; next }
  /^## / && in_ac { exit }
  in_ac { print }
' "$AC_BODY"

rm -f "$matched_files_tmp"
exit 0
