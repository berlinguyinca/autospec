#!/usr/bin/env bash
# scripts/classify-language.sh — deterministic language classifier for autospec issues.
#
# Usage:
#   scripts/classify-language.sh <body-file>          # emit "## Language fit" markdown block
#   scripts/classify-language.sh <body-file> --json   # emit JSON object
#   scripts/classify-language.sh --help               # show this help
#
# Output (default): ## Language fit markdown block between autospec-language markers
# Output (--json):  {"lang":"rust","source":"inherited","rationale":"...","deterministic":true,"confidence":0.95}
#
#   lang          closed label set: rust|go|python|typescript|javascript|java|bash|ruby|csharp|markdown|mixed|unknown
#   source        explicit | inherited | unknown   (repo-dominant and chosen arrive with ranks 3-5)
#   deterministic false only on a step-4 tie (ranks 3-5, not implemented here)
#
# Precedence (ranks 1-2, per docs/specs/2026-08-12-language-selection-axis-design.md):
#   rank 1  explicit-with-path — a language named within one line of a target path or a
#           create/add/touch/write <path> phrase; outranks inheritance
#   rank 2  inherited — ## Files touched paths resolve via extension to one language;
#           2+ distinct languages -> lang:mixed
#   neither -> lang:unknown, confidence 0.0 (abstention, never a guess)
#
# Telemetry: appends one JSON line per invocation to .autospec/telemetry/classify-language.jsonl
#
# Exit codes:
#   0  — success
#   1  — usage error / body file not found
#   2  — escalation failed (ranks 3-5, not implemented here)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: classify-language.sh <body-file> [--json]
       classify-language.sh --help

Deterministic language classifier for autospec issues (ranks 1-2).

Arguments:
  <body-file>    Path to the issue body file (markdown)

Options:
  --json         Emit a JSON object instead of the markdown block
  --help         Show this help and exit

Output (--json):
  {"lang":"rust","source":"inherited","rationale":"...","deterministic":true,"confidence":0.95}

Exit codes:
  0  success
  1  usage error / body file not found
  2  escalation failed (ranks 3-5, not implemented here)
EOF
}

info() { printf '%s\n' "$*" >&2; }
warn() { printf 'WARN: %s\n' "$*" >&2; }
err()  { printf 'ERROR: %s\n' "$*" >&2; }

JSON_MODE=0
BODY_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help)
      usage
      exit 0
      ;;
    --json)
      JSON_MODE=1
      ;;
    -*)
      err "unknown option: $1"
      usage >&2
      exit 1
      ;;
    *)
      if [ -z "$BODY_FILE" ]; then
        BODY_FILE="$1"
      else
        err "unexpected argument: $1"
        usage >&2
        exit 1
      fi
      ;;
  esac
  shift
done

if [ -z "$BODY_FILE" ]; then
  usage >&2
  exit 1
fi

if [ ! -f "$BODY_FILE" ]; then
  err "body file not found: $BODY_FILE"
  exit 1
fi

# ── helpers ────────────────────────────────────────────────────────────────────

# Config/markup-adjacent extensions (toml, json, yml, yaml, diff, patch) are
# deliberately NOT language evidence; the closed label set is inlined below.

# A rank-1 candidate line names a target: a backticked span with a slash, a
# slash path, a bare filename with a known extension, or a create/add/touch/write
# verb followed by a path-shaped token (must contain / or -).
has_path_token() {
  printf '%s' "$1" | grep -qE \
    "(\`[^\`]*\/[^\`]*\`)|([A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)+)|([A-Za-z0-9_.-]+\.(rs|go|py|ts|tsx|js|jsx|mjs|cjs|java|sh|bash|rb|cs|md|markdown)\b)|((create|add|touch|write)[[:space:]]+[A-Za-z0-9_./\`-]*[/-][A-Za-z0-9_./\`-]+)"
}

# Strip path tokens from a line, leaving prose for language-name matching.
# A language word embedded in a path (crates/rustc-helpers/src/lib.rs) must not
# count as an explicit naming.
pathless() {
  sed -E 's/`[^`]*`/ /g' \
    | sed -E 's/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]*/ /g' \
    | sed -E "s/[A-Za-z0-9_.-]*\.(rs|go|py|ts|tsx|js|jsx|mjs|cjs|java|sh|bash|rb|cs|md|markdown)\\b/ /g"
}

# Match a language name (word-bounded) in a pathless line. First match wins in
# a fixed order so the result is deterministic.
match_lang() {
  local line="$1" low
  low="$(printf '%s' "$line" | tr '[:upper:]' '[:lower:]')"
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])typescript([^a-z0-9]|$)'; then printf 'typescript'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])javascript([^a-z0-9]|$)'; then printf 'javascript'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])python([^a-z0-9]|$)'; then printf 'python'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])rust([^a-z0-9]|$)'; then printf 'rust'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])golang([^a-z0-9]|$)'; then printf 'go'; return 0; fi
  if printf '%s' "$line" | grep -qE '(^|[^A-Za-z0-9])Go([^A-Za-z0-9]|$)'; then printf 'go'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])java([^a-z0-9]|$)'; then printf 'java'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])ruby([^a-z0-9]|$)'; then printf 'ruby'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])(bash|shell)([^a-z0-9]|$)'; then printf 'bash'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])csharp([^a-z0-9]|$)|(^|[^a-z0-9])c#([^a-z0-9]|$)'; then printf 'csharp'; return 0; fi
  if printf '%s' "$low" | grep -qE '(^|[^a-z0-9])markdown([^a-z0-9]|$)'; then printf 'markdown'; return 0; fi
  return 1
}

# Rank 1: explicit-with-path. Prints "lang line_number" for the first qualifying
# line, checking the same line, then the previous line, then the next line.
rank1() {
  local body_file="$1"
  local n i j line w lang
  n="$(grep -c '' "$body_file")"
  for ((i = 1; i <= n; i++)); do
    line="$(sed -n "${i}p" "$body_file")"
    has_path_token "$line" || continue
    for j in "$i" $((i - 1)) $((i + 1)); do
      [ "$j" -lt 1 ] && continue
      [ "$j" -gt "$n" ] && continue
      w="$(sed -n "${j}p" "$body_file" | pathless)"
      if lang="$(match_lang "$w")"; then
        printf '%s %s\n' "$lang" "$j"
        return 0
      fi
    done
  done
  return 1
}

# Extract the ## Files touched section body (mirrors lint-issue.sh).
extract_files_touched() {
  awk '
    $0 == "## Files touched" { in_section = 1; next }
    in_section && /^## / { exit }
    in_section { print }
  ' "$1"
}

# First path-like token on a Files-touched line (backticked span, slash path, or
# filename with any extension). Empty when the line carries no path.
first_path_token() {
  printf '%s' "$1" \
    | grep -oE '`[^`]+`|[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)+|[A-Za-z0-9_.-]+\.[A-Za-z0-9]{1,10}' \
    | head -n 1 \
    | tr -d '`' || true
}

ext_to_lang() {
  case "$1" in
    rs) printf 'rust' ;;
    go) printf 'go' ;;
    py) printf 'python' ;;
    ts | tsx) printf 'typescript' ;;
    js | jsx | mjs | cjs) printf 'javascript' ;;
    java) printf 'java' ;;
    sh | bash) printf 'bash' ;;
    rb) printf 'ruby' ;;
    cs) printf 'csharp' ;;
    md | markdown) printf 'markdown' ;;
    *) return 0 ;;
  esac
}

# Rank 2: inherited. Resolves ## Files touched paths by extension.
# Sets RANK2_LANG (single language or "mixed"), RANK2_COUNT, RANK2_LIST.
# Returns 1 when no path resolves to a language.
rank2() {
  local body_file="$1"
  local line tok base ext lang
  local distinct=""
  RANK2_COUNT=0
  RANK2_LIST=""
  RANK2_LANG=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    tok="$(first_path_token "$line" || true)"
    [ -n "$tok" ] || continue
    base="${tok##*/}"
    case "$base" in
      *.*) ;;
      *) continue ;;
    esac
    ext="${base##*.}"
    ext="$(printf '%s' "$ext" | tr '[:upper:]' '[:lower:]')"
    lang="$(ext_to_lang "$ext")"
    [ -n "$lang" ] || continue
    case "$distinct" in
      *"|${lang}|"*) ;;
      *) distinct="${distinct}|${lang}|" ;;
    esac
  done < <(extract_files_touched "$body_file")
  [ -n "$distinct" ] || return 1
  local pipes
  pipes="$(printf '%s' "$distinct" | tr -cd '|' | wc -c)"
  RANK2_COUNT=$((pipes / 2))
  RANK2_LIST="${distinct#|}"
  RANK2_LIST="${RANK2_LIST%|}"
  RANK2_LIST="${RANK2_LIST//|/, }"
  if [ "$RANK2_COUNT" -eq 1 ]; then
    RANK2_LANG="${RANK2_LIST}"
  else
    RANK2_LANG="mixed"
  fi
  return 0
}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# ── classify ───────────────────────────────────────────────────────────────────

LANG_VAL="unknown"
SOURCE="unknown"
CONFIDENCE="0.0"
DETERMINISTIC=true
RATIONALE=""

if RANK1_RESULT="$(rank1 "$BODY_FILE")"; then
  LANG_VAL="${RANK1_RESULT%% *}"
  RANK1_LINE="${RANK1_RESULT##* }"
  SOURCE="explicit"
  CONFIDENCE="0.95"
  RATIONALE="Explicit: language named on line ${RANK1_LINE} alongside a target path (rank 1)"
elif rank2 "$BODY_FILE"; then
  if [ "$RANK2_COUNT" -eq 1 ]; then
    LANG_VAL="$RANK2_LANG"
    SOURCE="inherited"
    CONFIDENCE="0.95"
    RATIONALE="Inherited: every resolvable Files-touched path is ${LANG_VAL} (rank 2)"
  else
    LANG_VAL="mixed"
    SOURCE="inherited"
    CONFIDENCE="0.95"
    RATIONALE="Inherited: Files touched span ${RANK2_COUNT} languages (${RANK2_LIST}) (rank 2)"
  fi
else
  RATIONALE="Abstained: no explicit-with-path signal and no resolvable Files-touched language (ranks 1-2)"
fi

# ── telemetry ──────────────────────────────────────────────────────────────────

TELEMETRY_DIR="$REPO_ROOT/.autospec/telemetry"
mkdir -p "$TELEMETRY_DIR"
TELEMETRY_FILE="$TELEMETRY_DIR/classify-language.jsonl"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf '{"ts":"%s","deterministic":%s,"lang":"%s","source":"%s","confidence":%s}\n' \
  "$TS" "$DETERMINISTIC" "$LANG_VAL" "$SOURCE" "$CONFIDENCE" >> "$TELEMETRY_FILE"

# ── output ─────────────────────────────────────────────────────────────────────

if [ "$JSON_MODE" -eq 1 ]; then
  printf '{"lang":"%s","source":"%s","rationale":"%s","deterministic":%s,"confidence":%s}\n' \
    "$LANG_VAL" "$SOURCE" "$(json_escape "$RATIONALE")" "$DETERMINISTIC" "$CONFIDENCE"
else
  TODAY="$(date +%Y-%m-%d)"
  cat <<EOF
<!-- autospec-language:begin -->
## Language fit

- **Language:** \`lang:${LANG_VAL}\`
- **Source:** ${SOURCE}
- **Rationale:** ${RATIONALE}
- **Classified:** ${TODAY}

<!-- autospec-language:end -->
EOF
fi
