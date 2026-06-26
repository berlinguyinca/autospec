#!/usr/bin/env bash
# autonomous-persona-mine.sh — On the persona refresh cadence, mine the
# repo-slug-scoped session corpus for recurring decision patterns and emit a
# compact redacted precedence-3 digest consumed by F1
# (scripts/autonomous-persona-sources.sh).
#
# Usage:
#   bash scripts/autonomous-persona-mine.sh [--repo-root DIR] [--autospec-home DIR]
#
# Environment variables:
#   AUTOSPEC_PERSONA_MINE=0         Disable mining entirely (no-op exit 0)
#   AUTOSPEC_PERSONA_MINE_GLOBAL=1  Also mine all Claude projects corpora
#                                    (default: this repo's corpus only)
#   AUTOSPEC_PERSONA_MINE_TOP_N     Max patterns to emit (default: 20)
#   AUTOSPEC_PERSONA_EXTRACT_CMD    Subprocess seam: called as
#                                    <cmd> <corpus_dir>; overrides built-in
#                                    grep extractor (used for mocking in tests)
#   CLAUDE_PROJECTS_DIR             Override ~/.claude/projects path
#
# Output:
#   $AUTOSPEC_HOME/persona-mined-digest.json  (F1 precedence-3 source)
#
# Schema (persona-mined-digest/v1):
#   { schema, mined_at, corpus_scope, truncated, patterns:[{pattern,frequency}] }
#
# Engineering rules:
#   set -eu; if/then/fi (no one-sided && short-circuits); no RETURN traps;
#   jq --arg / --argjson for safe encoding; no test() with interpolated values.
#
# Exit codes:
#   0  Success or no-op (disabled / corpus absent)
#   1  Required tool (jq) not available

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
AUTOSPEC_HOME="${AUTOSPEC_HOME:-$HOME/.autospec}"
CLAUDE_PROJECTS_DIR="${CLAUDE_PROJECTS_DIR:-$HOME/.claude/projects}"
TOP_N="${AUTOSPEC_PERSONA_MINE_TOP_N:-20}"
REFRESH_DAYS="${AUTOSPEC_PERSONA_REFRESH_DAYS:-7}"
FORCE=0

# ── opt-out ────────────────────────────────────────────────────────────────
if [ "${AUTOSPEC_PERSONA_MINE:-1}" = "0" ]; then
  exit 0
fi

# ── argument parsing ───────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      if [ $# -lt 2 ]; then
        printf 'autonomous-persona-mine: --repo-root requires an argument\n' >&2
        exit 1
      fi
      REPO_ROOT="$2"
      shift 2
      ;;
    --autospec-home)
      if [ $# -lt 2 ]; then
        printf 'autonomous-persona-mine: --autospec-home requires an argument\n' >&2
        exit 1
      fi
      AUTOSPEC_HOME="$2"
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      grep '^#' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      printf 'autonomous-persona-mine: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

# ── dependency check ───────────────────────────────────────────────────────
if ! command -v jq >/dev/null 2>&1; then
  printf 'autonomous-persona-mine: jq not found — required for JSON output\n' >&2
  exit 1
fi

# ── cadence self-gate ──────────────────────────────────────────────────────
# Mining runs on the persona-refresh cadence, NOT every cycle. The conductor
# invokes this helper once per cycle; it fast-exits while the existing digest is
# younger than AUTOSPEC_PERSONA_REFRESH_DAYS, unless --force is given (used by an
# autospec:recalibrate-persona control signal). Mirrors persona-synth's gate.
_digest="$AUTOSPEC_HOME/persona-mined-digest.json"
if [ "$FORCE" -ne 1 ] && [ -f "$_digest" ]; then
  _now="$(date +%s)"
  if stat -f %m "$_digest" >/dev/null 2>&1; then
    _dmtime="$(stat -f %m "$_digest")"
  else
    _dmtime="$(stat -c %Y "$_digest" 2>/dev/null || echo 0)"
  fi
  _window_sec=$(( REFRESH_DAYS * 24 * 60 * 60 ))
  if [ $(( _now - _dmtime )) -lt "$_window_sec" ]; then
    exit 0
  fi
fi

# ── resolve repo-slug corpus path ─────────────────────────────────────────
# Convention: ~/.claude/projects/-Users-<whoami>-IdeaProjects-<repo>/memory
_whoami="$(whoami)"
_repo_name="$(basename "$REPO_ROOT")"
_repo_corpus="$CLAUDE_PROJECTS_DIR/-Users-${_whoami}-IdeaProjects-${_repo_name}/memory"

# ── collect corpus directories ────────────────────────────────────────────
_tmp_corpuslst="$(mktemp -t persona-mine-corpora.XXXXXX)"
trap 'rm -f "$_tmp_corpuslst"' EXIT

if [ -d "$_repo_corpus" ]; then
  printf '%s\n' "$_repo_corpus" >> "$_tmp_corpuslst"
fi

if [ "${AUTOSPEC_PERSONA_MINE_GLOBAL:-0}" = "1" ]; then
  if [ -d "$CLAUDE_PROJECTS_DIR" ]; then
    find "$CLAUDE_PROJECTS_DIR" -maxdepth 2 -name "memory" -type d 2>/dev/null \
      | while IFS= read -r _d; do
          # skip the repo-scoped entry already added
          if [ "$_d" = "$_repo_corpus" ]; then
            continue
          fi
          printf '%s\n' "$_d"
        done >> "$_tmp_corpuslst" || true
  fi
fi

# Fail-soft no-op when no corpus found
if [ ! -s "$_tmp_corpuslst" ]; then
  exit 0
fi

# ── built-in pattern extractor ────────────────────────────────────────────
# Extracts lines that look like imperative decision phrases from .md files.
_builtin_extract() {
  local _dir="$1"
  find "$_dir" -maxdepth 1 -name "*.md" -type f 2>/dev/null \
    | while IFS= read -r _f; do
        grep -iE \
          '^\s*[-*]?\s*(always|never|prefer|avoid|use|reject|require|must|should|do not|don'"'"'t|ensure|check|verify)' \
          "$_f" 2>/dev/null || true
      done
}

# ── gather raw patterns ───────────────────────────────────────────────────
_tmp_raw="$(mktemp -t persona-mine-raw.XXXXXX)"
trap 'rm -f "$_tmp_corpuslst" "$_tmp_raw"' EXIT

while IFS= read -r _cdir; do
  if [ ! -d "$_cdir" ]; then
    continue
  fi
  if [ -n "${AUTOSPEC_PERSONA_EXTRACT_CMD:-}" ]; then
    bash -c "$AUTOSPEC_PERSONA_EXTRACT_CMD \"\$1\"" _ "$_cdir" >> "$_tmp_raw" 2>/dev/null || true
  else
    _builtin_extract "$_cdir" >> "$_tmp_raw" 2>/dev/null || true
  fi
done < "$_tmp_corpuslst"

# ── redaction pass ────────────────────────────────────────────────────────
# Remove lines containing obvious secrets or absolute paths.
_tmp_redacted="$(mktemp -t persona-mine-redacted.XXXXXX)"
trap 'rm -f "$_tmp_corpuslst" "$_tmp_raw" "$_tmp_redacted"' EXIT

grep -viE \
  'password|passwd|secret|token|api[_-]?key|auth[_-]?key|bearer|credential|private[_-]?key|access[_-]?key|/Users/|/home/|/root/' \
  "$_tmp_raw" > "$_tmp_redacted" 2>/dev/null || true

# ── frequency count + top-N bounding ─────────────────────────────────────
_tmp_sorted="$(mktemp -t persona-mine-sorted.XXXXXX)"
trap 'rm -f "$_tmp_corpuslst" "$_tmp_raw" "$_tmp_redacted" "$_tmp_sorted"' EXIT

# Trim leading/trailing whitespace, count duplicates, sort descending
sed 's/^[[:space:]]*//' "$_tmp_redacted" \
  | sed 's/[[:space:]]*$//' \
  | grep -v '^$' \
  | sort \
  | uniq -c \
  | sort -rn \
  > "$_tmp_sorted" 2>/dev/null || true

_total="$(wc -l < "$_tmp_sorted" | tr -d ' ')"
_truncated="false"
if [ "$_total" -gt "$TOP_N" ]; then
  _truncated="true"
  printf 'autonomous-persona-mine: INFO truncating %s patterns to top %s\n' \
    "$_total" "$TOP_N" >&2
fi

# ── build patterns JSON array ─────────────────────────────────────────────
_tmp_patterns="$(mktemp -t persona-mine-patterns.XXXXXX)"
trap 'rm -f "$_tmp_corpuslst" "$_tmp_raw" "$_tmp_redacted" "$_tmp_sorted" "$_tmp_patterns"' EXIT

printf '[\n' > "$_tmp_patterns"
_first=1
_n=0
while IFS= read -r _line; do
  if [ "$_n" -ge "$TOP_N" ]; then
    break
  fi
  # Parse "   <count> <text>"
  _freq="$(printf '%s' "$_line" | awk '{print $1}')"
  _text="$(printf '%s' "$_line" | awk '{$1=""; sub(/^ /, ""); print}')"
  if [ -z "$_text" ]; then
    continue
  fi
  if [ "$_first" = "1" ]; then
    _first=0
  else
    printf ',\n' >> "$_tmp_patterns"
  fi
  jq -n --arg text "$_text" --argjson freq "${_freq:-1}" \
    '{pattern:$text,frequency:$freq}' >> "$_tmp_patterns"
  _n=$((_n + 1))
done < "$_tmp_sorted"
printf '\n]\n' >> "$_tmp_patterns"

_patterns_json="$(cat "$_tmp_patterns")"

# ── determine scope label ─────────────────────────────────────────────────
_scope="repo"
if [ "${AUTOSPEC_PERSONA_MINE_GLOBAL:-0}" = "1" ]; then
  _scope="global"
fi

# ── write digest ──────────────────────────────────────────────────────────
_mined_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date '+%Y-%m-%dT%H:%M:%SZ')"

mkdir -p "$AUTOSPEC_HOME"

jq -n \
  --arg   schema     "persona-mined-digest/v1" \
  --arg   mined_at   "$_mined_at" \
  --arg   scope      "$_scope" \
  --argjson truncated "$_truncated" \
  --argjson patterns  "$_patterns_json" \
  '{schema:$schema, mined_at:$mined_at, corpus_scope:$scope, truncated:$truncated, patterns:$patterns}' \
  > "$_digest"
