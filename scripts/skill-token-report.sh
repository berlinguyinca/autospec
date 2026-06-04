#!/usr/bin/env bash
# scripts/skill-token-report.sh — per-skill word+token report.
#
# Usage:
#   scripts/skill-token-report.sh                        # emit markdown table to stdout
#   scripts/skill-token-report.sh --json                 # emit JSON array to stdout
#   scripts/skill-token-report.sh --skills-dir <dir>     # override skills directory
#   scripts/skill-token-report.sh --help                 # show this help
#
# Token estimate: words * 133 / 100 (integer division, truncated).
#
# Output (default): markdown table with columns: Skill | Words | Tokens | Est. % dup
# Output (--json):  JSON array; each row: {"skill":"...","words":N,"tokens":N}
#
# Exit codes:
#   0  — success (even if skills dir is empty — returns empty table/array)
#   1  — usage error

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
JSON_MODE=0
SKILLS_DIR=""

usage() {
  cat <<'USAGE'
skill-token-report.sh — per-skill word+token report

Usage:
  scripts/skill-token-report.sh
  scripts/skill-token-report.sh --json
  scripts/skill-token-report.sh --skills-dir <dir>
  scripts/skill-token-report.sh --help

Token estimate: floor(words * 1.33)  [integer, no floats]

Exit codes:
  0  success
  1  usage error
USAGE
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --json)
      JSON_MODE=1
      shift
      ;;
    --skills-dir)
      if [ $# -lt 2 ]; then
        printf 'skill-token-report.sh: --skills-dir requires a directory argument\n' >&2
        exit 1
      fi
      SKILLS_DIR="$2"
      shift 2
      ;;
    -*)
      printf 'skill-token-report.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      printf 'skill-token-report.sh: unexpected argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

if [ -z "$SKILLS_DIR" ]; then
  SKILLS_DIR="$REPO_ROOT/skills"
fi

# ---------------------------------------------------------------------------
# Collect rows: skill name, words, tokens
# ---------------------------------------------------------------------------

# Each row stored as "skill<TAB>words<TAB>tokens"
rows=()

for skill_dir in "$SKILLS_DIR"/*/; do
  [ -d "$skill_dir" ] || continue
  skill_file="$skill_dir/SKILL.md"
  [ -f "$skill_file" ] || continue

  skill_name="$(basename "$skill_dir")"
  words=$(wc -w < "$skill_file")
  # Integer arithmetic: floor(words * 1.33) = words * 133 / 100
  tokens=$(( words * 133 / 100 ))

  rows+=("${skill_name}	${words}	${tokens}")
done

# ---------------------------------------------------------------------------
# Emit output
# ---------------------------------------------------------------------------

if [ "$JSON_MODE" -eq 1 ]; then
  # Emit JSON array
  printf '[\n'
  first=1
  for row in "${rows[@]+"${rows[@]}"}"; do
    skill_name="${row%%	*}"
    rest="${row#*	}"
    words_val="${rest%%	*}"
    tokens_val="${rest#*	}"
    if [ "$first" -eq 1 ]; then
      first=0
    else
      printf ',\n'
    fi
    printf '  {"skill":"%s","words":%s,"tokens":%s}' \
      "$skill_name" "$words_val" "$tokens_val"
  done
  printf '\n]\n'
else
  # Emit markdown table
  printf '| Skill | Words | Tokens |\n'
  printf '|-------|------:|-------:|\n'
  for row in "${rows[@]+"${rows[@]}"}"; do
    skill_name="${row%%	*}"
    rest="${row#*	}"
    words_val="${rest%%	*}"
    tokens_val="${rest#*	}"
    printf '| %s | %s | %s |\n' "$skill_name" "$words_val" "$tokens_val"
  done
fi
