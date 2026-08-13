#!/usr/bin/env bash
# scripts/gen-issue-skeleton.sh — render a structured YAML input into a team-lensed issue body.
#
# Usage:
#   scripts/gen-issue-skeleton.sh --input <file>   # read YAML from file
#   scripts/gen-issue-skeleton.sh                  # read YAML from stdin
#   scripts/gen-issue-skeleton.sh --help           # show this help
#
# Required YAML keys:
#   issue_id, spec_path, spec_url, goal_sentence,
#   team_personality (list), review_counter_team (list), files_to_read (list),
#   files_touched (list), local_llm_notes (list), dependencies (list),
#   implementation_scope (list), out_of_scope (list),
#   implementation_outline_lines (list), tests_required (list),
#   acceptance_criteria (list), verification.primary_smoke,
#   verification.operator_full, branch_name
# Optional profile: feature_profile: security_database additionally requires
#   evidence_consumed, controls_covered, prerequisites (lists)
#
# Output: structured markdown issue body on stdout.
# The output is piped through scripts/lint-issue.sh; non-zero exits propagate.
#
# Exit codes:
#   0   — success (lint passed)
#   1   — MISSING_FIELD or render error
#   N   — lint-issue.sh finding count

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LINT_BIN="$SCRIPT_DIR/lint-issue.sh"

usage() {
  cat <<'EOF'
gen-issue-skeleton.sh — render a structured YAML input into a team-lensed issue body

Usage:
  scripts/gen-issue-skeleton.sh --input <file>
  scripts/gen-issue-skeleton.sh < input.yaml
  scripts/gen-issue-skeleton.sh --help

Required YAML keys:
  issue_id, spec_path, spec_url, goal_sentence,
  team_personality, review_counter_team,
  files_to_read, files_touched, local_llm_notes, dependencies,
  implementation_scope, out_of_scope,
  implementation_outline_lines, tests_required, acceptance_criteria,
  verification.primary_smoke, verification.operator_full, branch_name
EOF
}

INPUT_FILE=""
for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    --input) ;;
    *) ;;
  esac
done

# Parse --input <file>
while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --input)
      if [ -z "${2:-}" ]; then
        printf 'gen-issue-skeleton.sh: --input requires a file argument\n' >&2
        exit 1
      fi
      INPUT_FILE="$2"
      shift 2
      ;;
    *)
      printf 'gen-issue-skeleton.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

# Read input: file or stdin
if [ -n "$INPUT_FILE" ]; then
  if [ ! -f "$INPUT_FILE" ]; then
    printf 'gen-issue-skeleton.sh: file not found: %s\n' "$INPUT_FILE" >&2
    exit 1
  fi
  YAML_CONTENT="$(cat "$INPUT_FILE")"
else
  YAML_CONTENT="$(cat)"
fi

# ── Minimal YAML parser (key-value + list extraction) ──────────────────────────
# Extracts simple scalar: yaml_get KEY
yaml_get() {
  local key="$1"
  printf '%s\n' "$YAML_CONTENT" | \
    awk -v key="$key" '
      $0 ~ "^"key":" {
        sub("^"key":[[:space:]]*", "")
        # Strip surrounding quotes
        gsub(/^["'"'"']|["'"'"']$/, "")
        print
        exit
      }
    '
}

# Extracts a list section (lines starting with "  - " or "- " after the key heading)
# Returns one item per line, stripped of leading "  - " or "- "
yaml_get_list() {
  local key="$1"
  printf '%s\n' "$YAML_CONTENT" | \
    awk -v key="$key" '
      /^[a-z]/ { in_section=0 }
      $0 ~ "^"key":" { in_section=1; next }
      in_section && /^[[:space:]]*-[[:space:]]/ {
        sub(/^[[:space:]]*-[[:space:]]/, "")
        # Strip surrounding quotes and braces
        gsub(/^["'"'"'{]|["'"'"'}]$/, "")
        # For path: ... entries, extract just the path value
        if ($0 ~ /^path:/) {
          sub(/^path:[[:space:]]*/, "")
          gsub(/^["'"'"']|["'"'"'],.*$/, "")
        }
        print
        next
      }
      in_section && /^[a-z]/ { exit }
    '
}

# Extracts nested scalar: yaml_get_nested PARENT CHILD
# e.g. yaml_get_nested verification primary_smoke
yaml_get_nested() {
  local parent="$1"
  local child="$2"
  printf '%s\n' "$YAML_CONTENT" | \
    awk -v parent="$parent" -v child="$child" '
      /^[a-z]/ { in_parent=0 }
      $0 ~ "^"parent":" { in_parent=1; next }
      in_parent && $0 ~ "^[[:space:]]+"child":" {
        sub(/^[[:space:]]+[a-z_]+:[[:space:]]*/, "")
        gsub(/^["'"'"']|["'"'"']$/, "")
        print
        exit
      }
    '
}

# ── Extract and validate required fields ───────────────────────────────────────
missing_field() {
  printf 'MISSING_FIELD:%s\n' "$1" >&2
  exit 1
}

ISSUE_ID="$(yaml_get issue_id)"
SPEC_PATH="$(yaml_get spec_path)"
SPEC_URL="$(yaml_get spec_url)"
GOAL_SENTENCE="$(yaml_get goal_sentence)"
BRANCH_NAME="$(yaml_get branch_name)"
FEATURE_PROFILE="$(yaml_get feature_profile)"
PRIMARY_SMOKE="$(yaml_get_nested verification primary_smoke)"
OPERATOR_FULL="$(yaml_get_nested verification operator_full)"

[ -n "$ISSUE_ID" ]       || missing_field "issue_id"
[ -n "$SPEC_PATH" ]      || missing_field "spec_path"
[ -n "$SPEC_URL" ]       || missing_field "spec_url"
[ -n "$GOAL_SENTENCE" ]  || missing_field "goal_sentence"
[ -n "$BRANCH_NAME" ]    || missing_field "branch_name"
[ -n "$PRIMARY_SMOKE" ]  || missing_field "verification.primary_smoke"

# Lists (rendered as bullet lists)
FILES_TO_READ="$(yaml_get_list files_to_read)"
FILES_TOUCHED="$(yaml_get_list files_touched)"
LOCAL_LLM_NOTES="$(yaml_get_list local_llm_notes)"
DEPENDENCIES="$(yaml_get_list dependencies)"
TEAM_PERSONALITY="$(yaml_get_list team_personality)"
REVIEW_COUNTER_TEAM="$(yaml_get_list review_counter_team)"
IMPL_SCOPE="$(yaml_get_list implementation_scope)"
OUT_OF_SCOPE="$(yaml_get_list out_of_scope)"
IMPL_OUTLINE="$(yaml_get_list implementation_outline_lines)"
TESTS_REQUIRED="$(yaml_get_list tests_required)"
ACCEPTANCE_CRITERIA="$(yaml_get_list acceptance_criteria)"

[ -n "$TEAM_PERSONALITY" ]   || missing_field "team_personality"
[ -n "$REVIEW_COUNTER_TEAM" ] || missing_field "review_counter_team"
[ -n "$FILES_TO_READ" ]       || missing_field "files_to_read"
[ -n "$FILES_TOUCHED" ]       || missing_field "files_touched"
[ -n "$LOCAL_LLM_NOTES" ]     || missing_field "local_llm_notes"
[ -n "$DEPENDENCIES" ]        || missing_field "dependencies"
[ -n "$IMPL_SCOPE" ]          || missing_field "implementation_scope"
[ -n "$IMPL_OUTLINE" ]        || missing_field "implementation_outline_lines"
[ -n "$TESTS_REQUIRED" ]      || missing_field "tests_required"
[ -n "$ACCEPTANCE_CRITERIA" ] || missing_field "acceptance_criteria"

SECURITY_CONTEXT=""
if [ "$FEATURE_PROFILE" = "security_database" ]; then
  EVIDENCE_CONSUMED="$(yaml_get_list evidence_consumed)"
  CONTROLS_COVERED="$(yaml_get_list controls_covered)"
  PREREQUISITES="$(yaml_get_list prerequisites)"
  [ -n "$EVIDENCE_CONSUMED" ] || missing_field "evidence_consumed"
  [ -n "$CONTROLS_COVERED" ]  || missing_field "controls_covered"
  [ -n "$PREREQUISITES" ]     || missing_field "prerequisites"
fi

# Format bullet list from newline-separated items
format_bullets() {
  while IFS= read -r line; do
    [ -n "$line" ] && printf -- '- %s\n' "$line"
  done
}

# Format numbered list from newline-separated items
format_numbered() {
  local n=1
  while IFS= read -r line; do
    if [ -n "$line" ]; then printf '%d. %s\n' "$n" "$line"; n=$((n + 1)); fi
  done
}

# Format acceptance criteria as checkboxes
format_checkboxes() {
  while IFS= read -r line; do
    [ -n "$line" ] && printf -- '- [ ] %s\n' "$line"
  done
}

format_lines() {
  while IFS= read -r line; do
    [ -n "$line" ] && printf '%s\n' "$line"
  done
}

if [ "$FEATURE_PROFILE" = "security_database" ]; then
  SECURITY_CONTEXT="$(cat <<MARKDOWN
## Evidence consumed

$(printf '%s\n' "$EVIDENCE_CONSUMED" | format_bullets)

## Controls covered

$(printf '%s\n' "$CONTROLS_COVERED" | format_bullets)

## Prerequisites

$(printf '%s\n' "$PREREQUISITES" | format_bullets)
MARKDOWN
)"
fi

# ── Render issue body template ────────────────────────────────────────────────
RENDERED_BODY="$(cat <<MARKDOWN
## Goal

${GOAL_SENTENCE}

## Source spec

\`${SPEC_PATH}\` — ${SPEC_URL}

## Team personality

$(printf '%s\n' "$TEAM_PERSONALITY" | format_bullets)

## Review counter-team

$(printf '%s\n' "$REVIEW_COUNTER_TEAM" | format_bullets)

## Files to read first

$(printf '%s\n' "$FILES_TO_READ" | format_bullets)

## Files touched

$(printf '%s\n' "$FILES_TOUCHED" | format_bullets)

## Local-LLM execution notes

$(printf '%s\n' "$LOCAL_LLM_NOTES" | format_bullets)

## Dependencies

$(printf '%s\n' "$DEPENDENCIES" | format_lines)

${SECURITY_CONTEXT}

## Implementation scope

$(printf '%s\n' "$IMPL_SCOPE" | format_bullets)

## Out of scope

$(printf '%s\n' "$OUT_OF_SCOPE" | format_bullets)

## Implementation outline

$(printf '%s\n' "$IMPL_OUTLINE" | format_numbered)

## Tests required

$(printf '%s\n' "$TESTS_REQUIRED" | format_bullets)

## Acceptance criteria

$(printf '%s\n' "$ACCEPTANCE_CRITERIA" | format_checkboxes)

## Verification

### Primary smoke test

\`\`\`
${PRIMARY_SMOKE}
\`\`\`

### Operator full

\`\`\`
${OPERATOR_FULL:-${PRIMARY_SMOKE}}
\`\`\`

## Branch name

\`${BRANCH_NAME}\`
MARKDOWN
)"

# ── Validate with lint-issue.sh ───────────────────────────────────────────────
TMP_BODY="$(mktemp)"
trap 'rm -f "$TMP_BODY"' EXIT
printf '%s\n' "$RENDERED_BODY" > "$TMP_BODY"

if [ -f "$LINT_BIN" ]; then
  lint_exit=0
  bash "$LINT_BIN" "$TMP_BODY" >&2 || lint_exit=$?
  if [ "$lint_exit" -ne 0 ]; then
    exit "$lint_exit"
  fi
fi

# Emit to stdout only after lint passes
printf '%s\n' "$RENDERED_BODY"
