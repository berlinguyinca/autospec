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
UPDATE_BASELINE=0
BASELINE_FILE="docs/reports/skill-token-baseline.md"

usage() {
  cat <<'USAGE'
skill-token-report.sh — per-skill word+token report

Usage:
  scripts/skill-token-report.sh
  scripts/skill-token-report.sh --json
  scripts/skill-token-report.sh --skills-dir <dir>
  scripts/skill-token-report.sh --update-baseline   # splice fresh table between
                                                    # <!-- baseline:begin/end --> markers in
                                                    # docs/reports/skill-token-baseline.md
                                                    # (header preserved; fails if markers missing)
  scripts/skill-token-report.sh --help

Token estimate: floor(words * 1.33)  [integer, no floats]

Exit codes:
  0  success
  1  usage error
  2  --update-baseline target or its markers missing
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
    --update-baseline)
      UPDATE_BASELINE=1
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

if [ "$JSON_MODE" -eq 1 ] && [ "$UPDATE_BASELINE" -eq 1 ]; then
  printf 'skill-token-report: ERROR — --json and --update-baseline are mutually exclusive\n' >&2
  exit 1
fi

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
  # Emit markdown table (to stdout, or spliced into the baseline file).
  emit_table() {
    printf '| Skill | Words | Tokens |\n'
    printf '|-------|------:|-------:|\n'
    for row in "${rows[@]+"${rows[@]}"}"; do
      skill_name="${row%%	*}"
      rest="${row#*	}"
      words_val="${rest%%	*}"
      tokens_val="${rest#*	}"
      printf '| %s | %s | %s |\n' "$skill_name" "$words_val" "$tokens_val"
    done
  }

  if [ "$UPDATE_BASELINE" -eq 1 ]; then
    # Splice the fresh table between the baseline markers, preserving the
    # header/footer. Fail closed (exit 2) when the file or markers are absent —
    # a bare `> baseline.md` redirect would destroy the marker wrapper the
    # validate.sh staleness gate parses.
    if [ ! -f "$BASELINE_FILE" ]; then
      printf 'skill-token-report: ERROR — %s not found; cannot --update-baseline\n' "$BASELINE_FILE" >&2
      exit 2
    fi
    if ! grep -q '<!-- baseline:begin -->' "$BASELINE_FILE" || ! grep -q '<!-- baseline:end -->' "$BASELINE_FILE"; then
      printf 'skill-token-report: ERROR — %s lacks <!-- baseline:begin/end --> markers; refusing to splice\n' "$BASELINE_FILE" >&2
      exit 2
    fi
    _table_tmp="$(mktemp)"
    _out_tmp="$(mktemp)"
    emit_table > "$_table_tmp"
    awk -v table="$_table_tmp" '
      /<!-- baseline:begin -->/ { print; while ((getline line < table) > 0) print line; close(table); skip=1; next }
      /<!-- baseline:end -->/   { skip=0 }
      skip != 1 { print }
    ' "$BASELINE_FILE" > "$_out_tmp"
    mv "$_out_tmp" "$BASELINE_FILE"
    rm -f "$_table_tmp"
    printf 'skill-token-report: baseline updated in place: %s\n' "$BASELINE_FILE"
  else
    emit_table
  fi
fi
