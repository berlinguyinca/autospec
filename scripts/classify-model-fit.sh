#!/usr/bin/env bash
# scripts/classify-model-fit.sh — deterministic ctx/reasoning classifier for autospec issues.
#
# Usage:
#   scripts/classify-model-fit.sh <body-file>         # emit ## Model fit block to stdout
#   scripts/classify-model-fit.sh <body-file> --json  # emit JSON object to stdout
#   scripts/classify-model-fit.sh --help              # show this help
#
# Environment:
#   LLM_ESCALATION_THRESHOLD  — confidence below this triggers LLM tie-breaker (default: 0.3)
#   AUTOSPEC_FORCE_LLM        — if set to 1, always use LLM path
#
# Output (default): ## Model fit markdown block
# Output (--json): {"ctx":"64k","reasoning":"medium","rationale":"...","deterministic":true|false,"confidence":0.0-1.0}
#
# Telemetry: appends one JSON line per invocation to .autospec/telemetry/classify-model-fit.jsonl
#
# Exit codes:
#   0  — success
#   1  — usage error or body file not found
#   2  — LLM escalation failed

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
THRESHOLD="${LLM_ESCALATION_THRESHOLD:-0.3}"
FORCE_LLM="${AUTOSPEC_FORCE_LLM:-0}"
JSON_MODE=0
BODY_FILE=""

usage() {
  cat <<'EOF'
classify-model-fit.sh — deterministic ctx/reasoning classifier for autospec issues

Usage:
  scripts/classify-model-fit.sh <body-file>
  scripts/classify-model-fit.sh <body-file> --json
  scripts/classify-model-fit.sh --help

Environment:
  LLM_ESCALATION_THRESHOLD  confidence below this triggers LLM (default: 0.3)
  AUTOSPEC_FORCE_LLM        set to 1 to always use LLM path

Exit codes:
  0  success
  1  usage error or body file not found
  2  LLM escalation failed
EOF
}

# Parse arguments
while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --json) JSON_MODE=1; shift ;;
    -*)
      printf 'classify-model-fit.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      if [ -z "$BODY_FILE" ]; then
        BODY_FILE="$1"
        shift
      else
        printf 'classify-model-fit.sh: unexpected argument: %s\n' "$1" >&2
        exit 1
      fi
      ;;
  esac
done

if [ -z "$BODY_FILE" ]; then
  printf 'classify-model-fit.sh: body file argument required\n' >&2
  usage >&2
  exit 1
fi

if [ ! -f "$BODY_FILE" ]; then
  printf 'classify-model-fit.sh: file not found: %s\n' "$BODY_FILE" >&2
  exit 1
fi

BODY="$(cat "$BODY_FILE")"

# ---------------------------------------------------------------------------
# score_ctx: returns ctx tier based on files_to_read count and anchor sizes
# ---------------------------------------------------------------------------
score_ctx() {
  local body="$1"

  # Count files_to_read lines: lines matching "- {path:" or "  - path:" or bullet list items
  # Also count simple "- `path`" or "- scripts/..." lines under "## Files to read first"
  local file_count=0
  local in_files_section=0
  local total_anchor_kb=0

  while IFS= read -r line; do
    # Detect section header
    if echo "$line" | grep -qE "^## Files to read"; then
      in_files_section=1
      continue
    fi
    # Exit files section on next ## header
    if echo "$line" | grep -qE "^## " && [ $in_files_section -eq 1 ]; then
      in_files_section=0
      continue
    fi
    if [ $in_files_section -eq 1 ]; then
      # Count bullet items (lines starting with - or *)
      if echo "$line" | grep -qE "^\s*[-*]\s+"; then
        file_count=$((file_count + 1))
        # Estimate anchor size: if anchor like §4 or §2.3 present, count as ~1kb each
        if echo "$line" | grep -qE "§[0-9]"; then
          total_anchor_kb=$((total_anchor_kb + 1))
        fi
      fi
    fi
  done <<< "$body"

  # Check for cross-skill markers
  local cross_skill=0
  if echo "$body" | grep -qiE "cross.skill|multi.skill|shared.*scripts|autospec-shared"; then
    cross_skill=1
  fi

  # Apply rubric
  if [ $file_count -le 3 ] && [ $total_anchor_kb -lt 1 ] && [ $cross_skill -eq 0 ]; then
    echo "32k"
  elif [ $file_count -le 7 ] && [ $total_anchor_kb -lt 5 ] && [ $cross_skill -eq 0 ]; then
    echo "64k"
  else
    echo "120k"
  fi
}

# ---------------------------------------------------------------------------
# score_reasoning: returns reasoning depth based on verbs in issue body
# ---------------------------------------------------------------------------
score_reasoning() {
  local body="$1"
  local body_lower
  body_lower="$(echo "$body" | tr '[:upper:]' '[:lower:]')"

  # Deep reasoning verbs
  if echo "$body_lower" | grep -qwE "design|reconcile|resolve|redesign|decide|architect|innovate"; then
    echo "deep"
    return
  fi

  # Shallow reasoning verbs (match only as action verbs, not nouns like "JSON format")
  # Use context: appears after "- " bullet or "## Goal" area, not as "output format" noun
  if echo "$body_lower" | grep -qE "(^|\s)(copy|rename|transcribe|mirror-exactly)\s" || \
     echo "$body_lower" | grep -qE "^\s*[-*]\s+(copy|rename|transcribe|list items|reformat)\b"; then
    echo "shallow"
    return
  fi

  # Medium reasoning verbs (default for adapt/integrate/wire/extend/mirror/follow)
  if echo "$body_lower" | grep -qwE "mirror|adapt|integrate|wire|follow|extend|implement|scaffold|add|create|generate"; then
    echo "medium"
    return
  fi

  # Default
  echo "medium"
}

# ---------------------------------------------------------------------------
# compute_confidence: returns a float 0.0-1.0 based on signal strength
# ---------------------------------------------------------------------------
compute_confidence() {
  local verbs_matched="$1"   # integer count of matched verb patterns
  local file_count_explicit="$2"  # integer
  local anchor_size_explicit="$3" # integer (anchor count)

  # Confidence formula:
  # - explicit file count > 0: +0.3
  # - explicit anchors > 0:    +0.2
  # - verbs matched >= 2:      +0.3
  # - verbs matched == 1:      +0.2
  # - base:                     0.1

  local conf_x10=1  # base 0.1 * 10

  if [ "$file_count_explicit" -gt 0 ]; then
    conf_x10=$((conf_x10 + 3))
  fi
  if [ "$anchor_size_explicit" -gt 0 ]; then
    conf_x10=$((conf_x10 + 2))
  fi
  if [ "$verbs_matched" -ge 2 ]; then
    conf_x10=$((conf_x10 + 3))
  elif [ "$verbs_matched" -ge 1 ]; then
    conf_x10=$((conf_x10 + 2))
  fi

  # Cap at 10 (= 1.0)
  if [ $conf_x10 -gt 10 ]; then
    conf_x10=10
  fi

  # Emit as decimal: integer / 10
  echo "0.${conf_x10}"
}

# ---------------------------------------------------------------------------
# count_verbs_matched: count how many verb categories matched
# ---------------------------------------------------------------------------
count_verbs_matched() {
  local body="$1"
  local body_lower
  body_lower="$(echo "$body" | tr '[:upper:]' '[:lower:]')"
  local count=0

  if echo "$body_lower" | grep -qwE "design|reconcile|resolve|redesign|decide|architect"; then
    count=$((count + 1))
  fi
  if echo "$body_lower" | grep -qE "(^|\s)(copy|rename|transcribe|mirror-exactly)\s"; then
    count=$((count + 1))
  fi
  if echo "$body_lower" | grep -qwE "mirror|adapt|integrate|wire|follow|extend|implement|scaffold|add|create|generate"; then
    count=$((count + 1))
  fi

  echo "$count"
}

# ---------------------------------------------------------------------------
# count_files_in_body: count files_to_read bullets
# ---------------------------------------------------------------------------
count_files_in_body() {
  local body="$1"
  local file_count=0
  local anchor_count=0
  local in_files_section=0

  while IFS= read -r line; do
    if echo "$line" | grep -qE "^## Files to read"; then
      in_files_section=1
      continue
    fi
    if echo "$line" | grep -qE "^## " && [ $in_files_section -eq 1 ]; then
      in_files_section=0
      continue
    fi
    if [ $in_files_section -eq 1 ]; then
      if echo "$line" | grep -qE "^\s*[-*]\s+"; then
        file_count=$((file_count + 1))
        if echo "$line" | grep -qE "§[0-9]"; then
          anchor_count=$((anchor_count + 1))
        fi
      fi
    fi
  done <<< "$body"

  echo "$file_count $anchor_count"
}

# ---------------------------------------------------------------------------
# llm_classify: escalate to LLM via omc ask
# ---------------------------------------------------------------------------
llm_classify() {
  local body="$1"
  local prompt="Classify this GitHub issue body for autospec model-fit.
Return JSON only: {\"ctx\":\"32k\"|\"64k\"|\"120k\",\"reasoning\":\"shallow\"|\"medium\"|\"deep\",\"rationale\":\"one sentence\"}

Issue body:
${body:0:3000}"

  local result
  if command -v omc &>/dev/null; then
    result="$(echo "$prompt" | omc ask 2>/dev/null)" || true
  fi

  if [ -z "${result:-}" ]; then
    # Fallback: return deterministic defaults as JSON
    printf '{"ctx":"64k","reasoning":"medium","rationale":"LLM unavailable; fallback to defaults"}'
    return 2
  fi

  # Extract JSON from result (omc may wrap in prose)
  local json
  json="$(echo "$result" | grep -o '{[^}]*}' | head -1)" || true
  if [ -z "$json" ]; then
    printf '{"ctx":"64k","reasoning":"medium","rationale":"LLM output unparseable; fallback"}'
    return 2
  fi

  echo "$json"
}

# ---------------------------------------------------------------------------
# Main classification logic
# ---------------------------------------------------------------------------

# Parse file count and anchor count
read -r FILE_COUNT ANCHOR_COUNT <<< "$(count_files_in_body "$BODY")"
VERBS_MATCHED="$(count_verbs_matched "$BODY")"

# Compute confidence
CONFIDENCE="$(compute_confidence "$VERBS_MATCHED" "$FILE_COUNT" "$ANCHOR_COUNT")"

# Compare confidence to threshold using awk
BELOW_THRESHOLD=0
if awk -v c="$CONFIDENCE" -v t="$THRESHOLD" 'BEGIN{exit !(c+0 < t+0)}' 2>/dev/null; then
  BELOW_THRESHOLD=1
fi

DETERMINISTIC=true
CTX=""
REASONING=""
RATIONALE=""

if [ "$FORCE_LLM" = "1" ] || [ "$BELOW_THRESHOLD" = "1" ]; then
  # LLM escalation path
  DETERMINISTIC=false
  LLM_RESULT="$(llm_classify "$BODY")" || LLM_EXIT=$?

  # Parse LLM JSON result
  CTX="$(echo "$LLM_RESULT" | grep -o '"ctx":"[^"]*"' | sed 's/"ctx":"//;s/"//')" || CTX="64k"
  REASONING="$(echo "$LLM_RESULT" | grep -o '"reasoning":"[^"]*"' | sed 's/"reasoning":"//;s/"//')" || REASONING="medium"
  RATIONALE="$(echo "$LLM_RESULT" | grep -o '"rationale":"[^"]*"' | sed 's/"rationale":"//;s/"//')" || RATIONALE="LLM classification"

  [ -z "$CTX" ] && CTX="64k"
  [ -z "$REASONING" ] && REASONING="medium"
  [ -z "$RATIONALE" ] && RATIONALE="LLM classification"
else
  # Deterministic path
  CTX="$(score_ctx "$BODY")"
  REASONING="$(score_reasoning "$BODY")"
  RATIONALE="Deterministic: ${FILE_COUNT} files-to-read, ${ANCHOR_COUNT} anchors, ${VERBS_MATCHED} verb categories matched (confidence ${CONFIDENCE})"
fi

# ---------------------------------------------------------------------------
# Telemetry append
# ---------------------------------------------------------------------------
TELEMETRY_DIR="$REPO_ROOT/.autospec/telemetry"
mkdir -p "$TELEMETRY_DIR"
TELEMETRY_FILE="$TELEMETRY_DIR/classify-model-fit.jsonl"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date +%Y-%m-%dT%H:%M:%SZ)"

printf '{"ts":"%s","deterministic":%s,"ctx":"%s","reasoning":"%s","confidence":%s}\n' \
  "$TS" "$DETERMINISTIC" "$CTX" "$REASONING" "$CONFIDENCE" \
  >> "$TELEMETRY_FILE"

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
if [ "$JSON_MODE" = "1" ]; then
  printf '{"ctx":"%s","reasoning":"%s","rationale":"%s","deterministic":%s,"confidence":%s}\n' \
    "$CTX" "$REASONING" "$RATIONALE" "$DETERMINISTIC" "$CONFIDENCE"
else
  # Emit ## Model fit block
  TODAY="$(date -u +%Y-%m-%d 2>/dev/null || date +%Y-%m-%d)"
  cat <<EOF
<!-- autospec-classify:begin -->
## Model fit

- **Context tier:** \`ctx:${CTX}\`
- **Reasoning depth:** \`reasoning:${REASONING}\`
- **Rationale:** ${RATIONALE}
- **Classified:** ${TODAY}

<!-- autospec-classify:end -->
EOF
fi
