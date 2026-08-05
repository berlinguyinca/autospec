#!/usr/bin/env bash
# scripts/lint-implementation-gates.sh — corrected size gates in front of
# lint-implementation.sh.
#
# Why this exists: COMPLEXITY told authors to split oversized files while PR_SIZE
# capped a commit at 400 changed lines, and a moved line counts twice. For a file of
# L lines the additions budget is (800 - L) / 2, so no file of 800+ lines could ever
# be brought under 400 by any sequence of commits — COMPLEXITY judges every
# intermediate commit on absolute size. See
# docs/superpowers/specs/2026-08-05-lint-gate-satisfiability-design.md.
#
# The gate also only ever ran as a local pre-commit hook, so oversized files grew
# freely through the merge path while local cleanup was rejected. It permitted the
# one direction that made the problem worse.
#
# This wrapper replaces two rules and delegates everything else untouched:
#
#   FILE_GROWTH   replaces the absolute file-LOC check. A file at or under the limit
#                 passes. A new file over the limit fails. An existing oversized file
#                 passes as long as it does not get longer.
#
#   PR_SIZE       its changed-line cap is waived for a pure shrink: no changed file
#                 gains lines and the net line count falls. A shrink cannot smuggle
#                 in a feature, because features add lines. The file-count and
#                 logical-unit caps still apply.
#
# Usage: lint-implementation-gates.sh [args...]     # args pass through unchanged
#
# Exit: 0 clean, 1 blocking findings remain, 2 invocation error.
#
# Env: AUTOSPEC_MAX_FILE_LOC (default 400)

set -eu

# 600, not 400: the limit is rough guidance meant to stop multi-thousand-line files,
# not to police a modest overage. A 500-600 line file that is cohesive is fine; the
# ratchet below is what actually holds the line, by refusing to let big files grow.
MAX_LOC="${AUTOSPEC_MAX_FILE_LOC:-600}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

DELEGATE="$SCRIPT_DIR/lint-implementation.sh"
if [ ! -f "$DELEGATE" ]; then
  DELEGATE="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh"
fi
if [ ! -f "$DELEGATE" ]; then
  echo "lint-implementation-gates.sh: cannot find lint-implementation.sh" >&2
  exit 2
fi

case "${1:-}" in
  -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

# Extensions the size rules never applied to.
is_exempt() {
  case "$1" in
    *.md|*.txt|*.json|*.yaml|*.yml|*.diff|*.lock) return 0 ;;
    *) return 1 ;;
  esac
}

staged_line_count() {  # staged_line_count <path> -> lines in the staged blob
  git show ":$1" 2>/dev/null | wc -l | tr -d ' '
}

head_line_count() {    # head_line_count <path> -> lines at HEAD, or "-" when new
  if git cat-file -e "HEAD:$1" 2>/dev/null; then
    git show "HEAD:$1" 2>/dev/null | wc -l | tr -d ' '
  else
    printf '%s' '-'
  fi
}

CHANGED="$(git diff --cached --name-only --diff-filter=ACMR 2>/dev/null || true)"

# ── the ratchet, as a filter ──────────────────────────────────────────────────
# Implemented by suppressing the delegate's absolute file-LOC finding for files that
# did not get longer, rather than by emitting a replacement finding. That keeps every
# existing suppression path — the per-issue `Guardian: skip-COMPLEXITY` opt-out and
# the inline `linter:allow` hatch — owned by the delegate. A separate rule id here
# would silently bypass those opt-outs.
NOT_GROWN=""
GREW=0
SHRANK=0
for f in $CHANGED; do
  [ -n "$f" ] || continue
  is_exempt "$f" && continue
  after="$(staged_line_count "$f")"
  before="$(head_line_count "$f")"
  [ -n "$after" ] || continue

  if [ "$before" = "-" ]; then
    # A new file has nothing to ratchet against; the delegate's limit stands.
    GREW=1
    continue
  fi

  if [ "$after" -gt "$before" ]; then
    GREW=1
  else
    NOT_GROWN="${NOT_GROWN}${f}
"
    [ "$after" -lt "$before" ] && SHRANK=1
  fi
done

# ── per-function cyclomatic complexity ────────────────────────────────────────
# The delegate's fallback counts decision keywords across a whole file and compares
# the total to a per-function threshold — its own comment says "per function". That
# punishes decomposition, which is the remedy the size rule prescribes: a module of
# eleven single-branch functions fails while one function with ten nested branches
# passes. Measure per function instead, and suppress the delegate's file-level
# finding only when no function actually exceeds the limit. Suppression stays with
# the delegate whenever a real violation exists, so its opt-outs keep working.
MAX_CC="${AUTOSPEC_MAX_CYCLOMATIC:-10}"
CC_CLEAN=""
CC_DETAIL=""
for f in $CHANGED; do
  case "$f" in *.py) ;; *) continue ;; esac
  [ -f "$f" ] || continue
  offenders="$(python3 - "$f" "$MAX_CC" <<'PY' 2>/dev/null || true
import ast, sys
DECISION = (ast.If, ast.For, ast.AsyncFor, ast.While, ast.ExceptHandler, ast.IfExp,
            ast.Assert, ast.With)
def cc(fn):
    n = 1
    for node in ast.walk(fn):
        if isinstance(node, DECISION):
            n += 1
        elif isinstance(node, ast.BoolOp):
            n += len(node.values) - 1
        elif isinstance(node, ast.comprehension):
            n += len(node.ifs)
    return n
try:
    tree = ast.parse(open(sys.argv[1], encoding="utf-8", errors="ignore").read())
except SyntaxError:
    sys.exit(0)
limit = int(sys.argv[2])
for node in ast.walk(tree):
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        score = cc(node)
        if score > limit:
            print(f"{node.lineno}:{node.name}:{score}")
PY
)"
  if [ -z "$offenders" ]; then
    CC_CLEAN="${CC_CLEAN}${f}
"
  else
    while IFS=: read -r ln name score; do
      [ -n "$name" ] || continue
      CC_DETAIL="${CC_DETAIL}INFO:COMPLEXITY:$f:$ln: function '${name}' has cyclomatic complexity ${score} (limit ${MAX_CC})
"
    done <<EOF
$offenders
EOF
  fi
done

# ── delegate, then drop the findings this wrapper supersedes ───────────────────
set +e
DELEGATE_OUT="$(bash "$DELEGATE" "$@" 2>&1)"
set -e

# A pure shrink: nothing grew, something did, and no new content was introduced.
IS_SHRINK=0
if [ "$GREW" -eq 0 ] && [ "$SHRANK" -eq 1 ]; then
  IS_SHRINK=1
fi

FILTERED="$(printf '%s\n' "$DELEGATE_OUT" | awk -v shrink="$IS_SHRINK" -v safe="$NOT_GROWN" -v ccok="$CC_CLEAN" '
  BEGIN {
    n = split(safe, rows, "\n"); for (i = 1; i <= n; i++) if (rows[i] != "") ok[rows[i]] = 1
    m = split(ccok, crows, "\n"); for (i = 1; i <= m; i++) if (crows[i] != "") ccclean[crows[i]] = 1
  }
  # File-wide keyword proxy dropped when no function actually exceeds the limit.
  /^COMPLEXITY:.*keyword-proxy cyclomatic/ {
    path = $0; sub(/^COMPLEXITY:/, "", path); sub(/:-:.*$/, "", path)
    if (path in ccclean) next
  }
  # Absolute file-LOC finding is dropped only for a file that did not get longer.
  /^COMPLEXITY:.*: file is [0-9]+ LOC/ {
    path = $0; sub(/^COMPLEXITY:/, "", path); sub(/:-:.*$/, "", path)
    if (path in ok) next
  }
  # Changed-line cap waived for a pure shrink; other PR_SIZE breaches still print.
  shrink == 1 && /PR_SIZE/ && /exceeded=changed_lines$/ { next }
  { print }
')"

OUT="$(printf '%s' "$FILTERED" | sed '/^$/d')"
if [ -n "$CC_DETAIL" ]; then
  OUT="${OUT:+$OUT
}$(printf '%s' "$CC_DETAIL" | sed '/^$/d')"
fi

[ -n "$OUT" ] && printf '%s\n' "$OUT"

# Blocking = any finding line that is not an INFO audit-trail entry.
BLOCKING="$(printf '%s\n' "$OUT" | grep -cE '^(ERROR:)?[A-Z][A-Z_]+:' || true)"
INFOS="$(printf '%s\n' "$OUT" | grep -cE '^INFO:' || true)"
if [ "$BLOCKING" -gt "$INFOS" ]; then
  exit 1
fi
exit 0
