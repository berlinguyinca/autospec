#!/usr/bin/env bash
# scripts/promote-eligibility.sh — deterministic fail-closed eligibility scorer
# for autospec backlog auto-grooming.
#
# Usage:
#   scripts/promote-eligibility.sh <body-file> --labels "<csv>"
#
# Environment:
#   GH_NONEXISTENT  — test hook; when set to 1, any "Depends on #N" reference
#                      is treated as pointing at a nonexistent issue.
#
# Output: JSON {"decision":"eligible|needs-template|epic|hold","reason":"..."}
#
#   eligible       — actionable body (>= MIN_BODY chars) AND a clear
#                     fix:/feat: intent-or-repro AND bounded scope
#                     (<= MAX_FILES referenced file paths, no epic marker).
#   epic           — has "epic" label or an epic marker in the body.
#   needs-template — groomable but not eligible (multi-file/complex).
#   hold           — ambiguous / unresolvable-dependency / too-thin.
#                     Fail-closed: anything uncertain lands here.
#
# Exit codes:
#   0  — success (decision emitted, including "hold")
#   1  — usage error or body file not found

set -eu

MIN_BODY=40
MAX_FILES=3

BODY_FILE=""
LABELS=""

usage() {
  cat <<'EOF'
promote-eligibility.sh — deterministic fail-closed eligibility scorer

Usage:
  scripts/promote-eligibility.sh <body-file> --labels "<csv>"

Environment:
  GH_NONEXISTENT  test hook: treat any "Depends on #N" as nonexistent

Exit codes:
  0  success
  1  usage error or body file not found
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --labels)
      LABELS="${2:-}"
      shift 2
      ;;
    -*)
      printf 'promote-eligibility.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      if [ -z "$BODY_FILE" ]; then
        BODY_FILE="$1"
        shift
      else
        printf 'promote-eligibility.sh: unexpected argument: %s\n' "$1" >&2
        exit 1
      fi
      ;;
  esac
done

if [ -z "$BODY_FILE" ]; then
  printf 'promote-eligibility.sh: body file argument required\n' >&2
  usage >&2
  exit 1
fi

if [ ! -f "$BODY_FILE" ]; then
  printf 'promote-eligibility.sh: file not found: %s\n' "$BODY_FILE" >&2
  exit 1
fi

BODY="$(cat "$BODY_FILE")"
BODY_LOWER="$(printf '%s' "$BODY" | tr '[:upper:]' '[:lower:]')"
LABELS_LOWER="$(printf '%s' "$LABELS" | tr '[:upper:]' '[:lower:]')"

emit() {
  decision="$1"
  reason="$2"
  # Escape backslashes and double-quotes for safe JSON string embedding.
  reason_json="$(printf '%s' "$reason" | sed 's/\\/\\\\/g; s/"/\\"/g')"
  printf '{"decision":"%s","reason":"%s"}\n' "$decision" "$reason_json"
}

# ---------------------------------------------------------------------------
# 1. Epic detection: "epic" label, or an epic marker in the body.
# ---------------------------------------------------------------------------
csv_has_label() {
  # $1 = comma-separated list (lowercased), $2 = label to find (lowercased)
  case ",${1}," in
    *",${2},"*) return 0 ;;
    *) return 1 ;;
  esac
}

if csv_has_label "$LABELS_LOWER" "epic"; then
  emit "epic" "epic label present"
  exit 0
fi

case "$BODY_LOWER" in
  *multi-subsystem*|*"multi subsystem"*)
    emit "epic" "epic marker: multi-subsystem scope in body"
    exit 0
    ;;
esac

# ---------------------------------------------------------------------------
# 2. Unresolvable dependency: "Depends on #N" where N does not exist.
# ---------------------------------------------------------------------------
DEP_NUM=""
DEP_NUM="$(printf '%s' "$BODY" | grep -Eio 'depends on #[0-9]+' | head -1 | grep -Eo '[0-9]+' || true)"

if [ -n "$DEP_NUM" ]; then
  DEP_EXISTS=0
  if [ "${GH_NONEXISTENT:-0}" = "1" ]; then
    DEP_EXISTS=0
  elif command -v gh >/dev/null 2>&1; then
    if gh issue view "$DEP_NUM" >/dev/null 2>&1; then
      DEP_EXISTS=1
    else
      DEP_EXISTS=0
    fi
  else
    # No way to verify: fail closed.
    DEP_EXISTS=0
  fi

  if [ "$DEP_EXISTS" -ne 1 ]; then
    emit "hold" "unresolvable dependency: issue #${DEP_NUM} referenced in body could not be verified to exist"
    exit 0
  fi
fi

# ---------------------------------------------------------------------------
# 3. Too-thin check: body must meet a minimum actionable length.
# ---------------------------------------------------------------------------
BODY_LEN="${#BODY}"
if [ "$BODY_LEN" -lt "$MIN_BODY" ]; then
  emit "hold" "too thin: body is ${BODY_LEN} chars, below MIN_BODY=${MIN_BODY}"
  exit 0
fi

# ---------------------------------------------------------------------------
# 4. Clear fix:/feat: intent-or-repro check.
# ---------------------------------------------------------------------------
HAS_INTENT=0
case "$BODY_LOWER" in
  fix:*|feat:*|*" fix:"*|*" feat:"*)
    HAS_INTENT=1
    ;;
esac
if [ "$HAS_INTENT" -eq 0 ]; then
  case "$BODY_LOWER" in
    *repro:*|*"steps to reproduce"*|*"expected:"*)
      HAS_INTENT=1
      ;;
  esac
fi

if [ "$HAS_INTENT" -eq 0 ]; then
  emit "hold" "ambiguous: no clear fix/feat intent or repro found in body"
  exit 0
fi

# ---------------------------------------------------------------------------
# 5. Bounded scope: count distinct file-path-looking tokens.
# ---------------------------------------------------------------------------
FILE_COUNT=0
FILE_COUNT="$(printf '%s' "$BODY" | grep -Eo '[A-Za-z0-9_./-]+\.[A-Za-z]{1,5}' | sort -u | grep -c . || true)"
if [ -z "$FILE_COUNT" ]; then
  FILE_COUNT=0
fi

if [ "$FILE_COUNT" -le "$MAX_FILES" ]; then
  emit "eligible" "actionable body with clear fix/feat intent and bounded scope (${FILE_COUNT} file path(s) referenced)"
  exit 0
fi

emit "needs-template" "groomable but scope too broad (${FILE_COUNT} file paths referenced, exceeds MAX_FILES=${MAX_FILES})"
exit 0
