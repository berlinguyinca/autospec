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
#   source        explicit | inherited | repo-dominant | chosen | unknown
#   deterministic false only when a rank-5 tie reaches a Tier-B (omc) call
#
# Precedence (ranks 1-5, per docs/specs/2026-08-12-language-selection-axis-design.md):
#   rank 1  explicit-with-path — a language named within one line of a target path or a
#           create/add/touch/write <path> phrase; outranks inheritance
#   rank 2  inherited — ## Files touched paths resolve via extension to one language;
#           2+ distinct languages -> lang:mixed
#   rank 3  explicit-prose — the body names exactly one canonical language in prose
#   rank 4  repo-dominant — the repo's dominant language at confidence > 0.5
#   rank 5  chosen — a spec-table row matches the body; ties resolve by repo affinity,
#           then the operator default, then a single Tier-B (omc) call
#   none    -> lang:unknown, confidence 0.0 (abstention, never a guess)
#
# Telemetry: appends one JSON line per invocation to .autospec/telemetry/classify-language.jsonl
#
# Exit codes:
#   0  — success
#   1  — usage error / body file not found
#   2  — escalation failed (Tier-B tie-break unavailable)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
CONFIG_FILE="${AUTOSPEC_CONFIG_FILE:-$REPO_ROOT/.autospec/autospec.yml}"
EXIT_CODE=0

usage() {
  cat <<'EOF'
Usage: classify-language.sh <body-file> [--json]
       classify-language.sh --help

Deterministic language classifier for autospec issues (ranks 1-5).

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
  2  escalation failed (Tier-B tie-break unavailable)
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

BODY="$(cat "$BODY_FILE")"

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

# Count comma-separated entries in a csv string (empty -> 0).
count_csv() {
  local s="${1:-}" commas
  [ -n "$s" ] || { printf '0'; return 0; }
  commas="$(printf '%s' "$s" | tr -cd ',' | wc -c)"
  printf '%s' $((commas + 1))
}

# Map a lowercased alphabetic token to a canonical closed-set language.
# Emits nothing for non-language tokens.
prose_lang() {
  local t="${1:-}"
  case "$t" in
    typescript | ts) printf 'typescript' ;;
    javascript | js) printf 'javascript' ;;
    python) printf 'python' ;;
    rust) printf 'rust' ;;
    golang | go) printf 'go' ;;
    java) printf 'java' ;;
    ruby) printf 'ruby' ;;
    bash | shell | sh) printf 'bash' ;;
    csharp) printf 'csharp' ;;
    markdown | md) printf 'markdown' ;;
    *) return 0 ;;
  esac
}

# Rank 3: explicit-prose. Exactly one canonical language named in the body
# (path tokens stripped). Sets RANK3_LANG.
RANK3_LANG=""
rank3() {
  local w tok lang distinct=""
  set -f
  for w in $(printf '%s\n' "$BODY" | pathless); do
    tok="$(printf '%s' "$w" | tr -cd '[:alpha:]' | tr '[:upper:]' '[:lower:]')"
    [ -n "$tok" ] || continue
    lang="$(prose_lang "$tok")"
    [ -n "$lang" ] || continue
    case "$distinct" in
      *"|${lang}|"*) ;;
      *) distinct="${distinct}|${lang}|" ;;
    esac
  done
  set +f
  local pipes
  pipes="$(printf '%s' "$distinct" | tr -cd '|' | wc -c)"
  [ $((pipes / 2)) -eq 1 ] || return 1
  RANK3_LANG="${distinct#|}"
  RANK3_LANG="${RANK3_LANG%|}"
  return 0
}

# Rank 4: repo-dominant. Always well-formed: "<lang|- > <conf> [<csv>]".
REPO_DOM="- 0.0"
repo_dominant_call() {
  REPO_DOM="$(bash "$SCRIPT_DIR/autospec-language-table.sh" repo_dominant "$REPO_ROOT" 2>/dev/null)" || REPO_DOM="- 0.0"
}

# Rank 5: chosen. Match the lowercased body against the spec's 6-row table;
# sets CHOSEN_CANDS to a sorted comma list of candidate languages.
CHOSEN_CANDS=""
score_chosen_rows() {
  local low cands=" " l w canon extracted
  low="$(printf '%s' "$BODY" | tr '[:upper:]' '[:lower:]')"
  if printf '%s' "$low" | grep -qE 'single binary|no runtime dependency|distributed to users'; then
    for l in go rust; do
      case "$cands" in *" $l "*) ;; *) cands="$cands $l" ;; esac
    done
  fi
  if printf '%s' "$low" | grep -qE 'hot loop|parser|memory[- ]bound|gc[- ]pause|must not gc'; then
    case "$cands" in *" rust "*) ;; *) cands="$cands rust" ;; esac
  fi
  if printf '%s' "$low" | grep -qE 'glue over|200 lines|<=200|posix'; then
    case "$cands" in *" bash "*) ;; *) cands="$cands bash" ;; esac
  fi
  if printf '%s' "$low" | grep -qE 'web ui|browser'; then
    case "$cands" in *" typescript "*) ;; *) cands="$cands typescript" ;; esac
  fi
  if printf '%s' "$low" | grep -qE '\bml\b|dataframe|scientific'; then
    case "$cands" in *" python "*) ;; *) cands="$cands python" ;; esac
  fi
  if printf '%s' "$low" | grep -qE 'librar|sdk'; then
    extracted="$(printf '%s' "$low" | grep -oE '\b(golang|typescript|javascript|python|rust|ruby|bash|csharp|java|go)\b' || true)"
    for w in $extracted; do
      canon="$(prose_lang "$w")"
      [ -n "$canon" ] || continue
      case "$cands" in *" $canon "*) ;; *) cands="$cands $canon" ;; esac
    done
  fi
  [ -n "${cands// /}" ] || return 1
  CHOSEN_CANDS="$(printf '%s\n' $cands | awk 'NF' | LC_ALL=C sort -u | paste -sd, -)"
  return 0
}

# Operator default from .autospec/autospec.yml ("language: <lang>"),
# canonicalized; empty when absent or outside the closed set.
operator_default() {
  [ -f "$CONFIG_FILE" ] || return 0
  local v
  v="$(sed -nE 's/^[[:space:]]*language:[[:space:]]*([A-Za-z]+).*/\1/p' "$CONFIG_FILE" | head -n 1 || true)"
  [ -n "$v" ] || return 0
  prose_lang "$(printf '%s' "$v" | tr '[:upper:]' '[:lower:]')"
}

# Tier-B (omc) tie-break: at most one call. Sets CHOSEN_LLM_LANG when omc
# replies with one of the candidates.
CHOSEN_LLM_LANG=""
llm_tiebreak() {
  local cands="$1" prompt raw canon t
  command -v omc >/dev/null 2>&1 || return 1
  prompt="$(printf '%s' "$BODY" | head -c 3000 || true)"
  raw="$(printf '%s\nCANDIDATE LANGUAGES: %s\nPick exactly one. Reply with the language name only.\n' \
    "$prompt" "$cands" | omc ask 2>/dev/null | head -n 1 || true)"
  [ -n "$raw" ] || return 1
  canon="$(prose_lang "$(printf '%s' "$raw" | tr -cd '[:alpha:]' | tr '[:upper:]' '[:lower:]')")"
  [ -n "$canon" ] || return 1
  set -f
  for t in ${cands//,/ }; do
    if [ "$t" = "$canon" ]; then
      CHOSEN_LLM_LANG="$canon"
      set +f
      return 0
    fi
  done
  set +f
  return 1
}

# Tie-break chain: repo-affinity (exactly one candidate hit) -> operator
# default (must be among candidates) -> Tier-B. Sets CHOSEN_LANG/CHOSEN_SOURCE.
CHOSEN_LANG=""
CHOSEN_SOURCE=""
resolve_tie() {
  local cands="$1" affinity="${2:-}"
  local aff=",$affinity," pool=",$cands," t op
  local hit_count=0
  set -f
  for t in ${cands//,/ }; do
    case "$aff" in
      *",$t,"*) hit_count=$((hit_count + 1)) ;;
    esac
  done
  if [ "$hit_count" -eq 1 ]; then
    for t in ${cands//,/ }; do
      case "$aff" in
        *",$t,"*) CHOSEN_LANG="$t" ;;
      esac
    done
    CHOSEN_SOURCE="repo-affinity"
    set +f
    return 0
  fi
  op="$(operator_default)"
  if [ -n "$op" ]; then
    case "$pool" in
      *",$op,"*)
        CHOSEN_LANG="$op"
        CHOSEN_SOURCE="operator default"
        set +f
        return 0
        ;;
    esac
  fi
  set +f
  if llm_tiebreak "$cands"; then
    CHOSEN_LANG="$CHOSEN_LLM_LANG"
    CHOSEN_SOURCE="tier-b"
    return 0
  fi
  return 1
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
  REPO_AFFINITY=""
  if rank3; then
    LANG_VAL="$RANK3_LANG"
    SOURCE="explicit"
    CONFIDENCE="0.8"
    RATIONALE="Explicit-prose: body names exactly one language, ${LANG_VAL} (rank 3)"
  else
    repo_dominant_call
    read -r rd_lang rd_conf rd_csv <<< "$REPO_DOM"
    REPO_AFFINITY="${rd_csv:-}"
    if [ "$rd_lang" != "-" ]; then
      LANG_VAL="$rd_lang"
      SOURCE="repo-dominant"
      CONFIDENCE="$rd_conf"
      RATIONALE="Repo-dominant: ${LANG_VAL} dominates the repo at ${rd_conf} (rank 4)"
    elif score_chosen_rows; then
      if [ "$(count_csv "$CHOSEN_CANDS")" -eq 1 ]; then
        LANG_VAL="$CHOSEN_CANDS"
        SOURCE="chosen"
        CONFIDENCE="0.7"
        RATIONALE="Chosen: unique spec-table row match, ${LANG_VAL} (rank 5)"
      elif resolve_tie "$CHOSEN_CANDS" "$REPO_AFFINITY"; then
        LANG_VAL="$CHOSEN_LANG"
        SOURCE="chosen"
        if [ "$CHOSEN_SOURCE" = "tier-b" ]; then
          CONFIDENCE="0.6"
          DETERMINISTIC=false
        else
          CONFIDENCE="0.7"
        fi
        RATIONALE="Chosen: rank-5 tie among (${CHOSEN_CANDS}) broken by ${CHOSEN_SOURCE} -> ${LANG_VAL}"
      else
        LANG_VAL="unknown"
        SOURCE="unknown"
        CONFIDENCE="0.0"
        DETERMINISTIC=false
        EXIT_CODE=2
        RATIONALE="Abstained: rank-5 tie among (${CHOSEN_CANDS}) unresolved, Tier-B unavailable"
      fi
    else
      LANG_VAL="unknown"
      SOURCE="unknown"
      CONFIDENCE="0.0"
      RATIONALE="Abstained: no explicit-with-path, Files-touched, prose, repo-dominant, or spec-table signal (ranks 1-5)"
    fi
  fi
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

exit "$EXIT_CODE"
