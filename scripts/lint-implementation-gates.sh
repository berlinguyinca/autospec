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
# This wrapper corrects two rules and delegates everything else untouched:
#
#   file LOC      becomes a ratchet. A file at or under the limit passes. An existing
#                 oversized file passes as long as it does not get longer. A new file
#                 over the limit fails, unless it holds relocated code — see the
#                 relocation note below.
#
#   PR_SIZE       its changed-line cap is waived for a pure shrink: no pre-existing
#                 file gains lines and the change removes at least as many lines as it
#                 adds. A shrink cannot smuggle in a feature, because features add
#                 lines. The file-count and logical-unit caps still apply.
#
# Usage: lint-implementation-gates.sh [args...]     # args pass through unchanged
#
# Exit: 0 clean, 1 blocking findings remain, 2 invocation error.
#
# Env: AUTOSPEC_MAX_FILE_LOC (default 600), AUTOSPEC_MAX_CYCLOMATIC (default 10)

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
# did not get longer, rather than by emitting a replacement finding. That keeps the
# per-issue `Guardian: skip-COMPLEXITY` opt-out — the only one that reaches this
# finding, since it is emitted with no line number and so no inline `linter:allow`
# comment can attach to it — owned by the delegate. A separate rule id here would
# silently bypass it.
NOT_GROWN=""
NEW_FILES=""
NEW_OVERSIZED=""
PREEXISTING_GREW=0
SHRANK=0
for f in $CHANGED; do
  [ -n "$f" ] || continue
  is_exempt "$f" && continue
  after="$(staged_line_count "$f")"
  before="$(head_line_count "$f")"
  [ -n "$after" ] || continue

  if [ "$before" = "-" ]; then
    # A new file has nothing to ratchet against. Judged below: against the limit
    # outright, unless the change as a whole is a relocation.
    NEW_FILES="${NEW_FILES}${f}
"
    if [ "$after" -gt "$MAX_LOC" ]; then
      NEW_OVERSIZED="${NEW_OVERSIZED}${f}
"
    fi
    continue
  fi

  if [ "$after" -gt "$before" ]; then
    PREEXISTING_GREW=1
  else
    NOT_GROWN="${NOT_GROWN}${f}
"
    [ "$after" -lt "$before" ] && SHRANK=1
  fi
done

# ── relocation ────────────────────────────────────────────────────────────────
# Extracting code out of a monolith into a sibling module is the remedy the size rule
# prescribes, and a split necessarily creates a file. Counting any new file as growth
# therefore rejects the prescribed remedy, which is the §1.1 defect again: the gate
# permits the direction that makes things worse and blocks the one that helps.
#
# A change that removes at least as many lines as it adds is a relocation, not new
# material — a feature cannot hide inside one, because features add lines. The delta
# comes from --numstat rather than from differencing file lengths, because git reports
# a move as a rename: the source path never appears on its own, so length differencing
# counts the destination as wholly new and a pure move reads as pure addition.
#
# Counted over the diff body rather than --numstat, and over CODE lines only. An extraction
# cannot avoid adding a module declaration, an import and a header explaining the split, so
# counting those made every honest extraction read as net growth and forfeited the waiver.
# Measured on the real case: lifting the harness cluster out of executor_bridge.rs moved 595
# lines out and 626 in — a raw net of +31, of which the code-line net was exactly 0.
#
# A feature cannot hide in a comment, a blank line or a `mod`/`use`/`import` declaration, so
# excluding them costs the rule none of what it protects. Statements still count, which is
# what stops the waiver laundering a feature through a refactor.
NET_DELTA="$(git diff --cached -U0 2>/dev/null | awk '
  # Track the file each hunk belongs to, so documentation and data stay exempt exactly as
  # they were under --numstat. Dropping that exemption would make a docs-heavy change read as
  # growth and quietly forfeit a waiver it should have kept.
  /^\+\+\+ / {
    path = $2
    sub(/^b\//, "", path)
    skip = (path ~ /\.(md|txt|json|ya?ml|diff|lock)$/)
    next
  }
  /^--- / { next }
  skip { next }
  /^(\+|-)/ {
    sign = substr($0, 1, 1)
    body = substr($0, 2)
    stripped = body
    sub(/^[[:space:]]+/, "", stripped)
    if (stripped == "") next                                   # blank
    if (stripped ~ /^(\/\/|#|\*|\/\*|--|;;)/) next             # comment, any of the usual markers
    if (stripped ~ /^(pub[(][a-z]+[)] )?(mod|use|import|export|from|require|package) /) next
    if (sign == "+") net += 1; else net -= 1
  }
  END { print net + 0 }
')"
IS_RELOCATION=0
MOVED=""
MOVED_ADDS=""
if [ "$PREEXISTING_GREW" -eq 0 ] && [ "${NET_DELTA:-0}" -le 0 ]; then
  IS_RELOCATION=1
  MOVED="$NEW_OVERSIZED"
  # Every new file in a net-removing change holds relocated content, whether or not
  # it lands over the limit — so the delegate's separate "file adds N lines" rule
  # needs the same treatment. That rule has its own hard-coded 500-line threshold, so
  # a 590-line extraction trips it even when it is comfortably under MAX_LOC.
  MOVED_ADDS="$NEW_FILES"
fi

# ── logical units, excluding docs ─────────────────────────────────────────────
# DOC_OUT_OF_SYNC requires any public-surface change — a flag, an env var, an exported
# function, a config key — to touch a doc file. PR_SIZE then charged its three-unit cap
# for that same file, so a change sitting at the limit failed the moment it was
# documented and the only way to satisfy both rules was to leave the change
# undocumented. Recount without docs and drop the breach when the real count fits.
#
# Docs still count toward the changed-line and raw-file caps, so a large documentation
# change is not free. 3 mirrors PR_SIZE_MAX_UNITS in the delegate.
MAX_UNITS=3
UNITS_NO_DOCS="$(
  for f in $CHANGED; do
    [ -n "$f" ] || continue
    case "$f" in
      tests/fixtures/skill-goldens/*.sha256) continue ;;
      # Before the doc case: a skill's SKILL.md is also a doc file, and a trio must
      # collapse to one unit rather than drop out of the count entirely.
      skills/*/SKILL.md|skills/*/codex/prompt.md|skills/*/opencode/agent.md)
        printf 'skills/%s/<trio>\n' "$(printf '%s' "$f" | cut -d/ -f2)" ;;
      README*|AGENTS.md|docs/*|*/SKILL.md|SKILL.md) continue ;;
      *) printf '%s\n' "$f" ;;
    esac
  done | sort -u | sed '/^$/d' | wc -l | tr -d ' '
)"
# Requires a non-empty staged file list: in --diff-file and PR modes this wrapper has no
# staged paths to count, and an empty count must not read as "within cap".
UNITS_OK=0
if [ -n "$CHANGED" ] && [ "${UNITS_NO_DOCS:-0}" -le "$MAX_UNITS" ]; then
  UNITS_OK=1
fi

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
# MAX_LOC is exported rather than merely read: the delegate has its own default, so
# without this the threshold documented here would have no effect and the local gate
# would disagree with the CI ratchet in .github/workflows/file-size-ratchet.yml.
set +e
DELEGATE_OUT="$(AUTOSPEC_MAX_FILE_LOC="$MAX_LOC" bash "$DELEGATE" "$@" 2>&1)"
DELEGATE_RC=$?
set -e

# The delegate's exit contract is a blocking-finding count capped at 64, or 200 for a
# scope explosion. Anything else means it did not run to completion — a crash, a missing
# interpreter, a syntax error — and its findings are then absent rather than clean.
# Judging the run by stdout alone scores that as a pass: a recursive emit helper in an
# earlier draft of the advisory routing segfaulted the delegate, and this wrapper
# reported a clean gate for every rule at once. Same failure shape as the mawk panic
# that made lint-ui.sh report every file clean, so it fails loud here too.
delegate_blocking="$(printf '%s\n' "$DELEGATE_OUT" \
  | grep -E '^(ERROR:)?[A-Z][A-Z_]+:' \
  | grep -cv '^INFO:' || true)"
#
# Compared as `rc <= printed`, not `rc == printed`, and the asymmetry is the whole point.
# A crash can only TRUNCATE output, never manufacture extra finding lines, so a delegate
# reporting fewer findings than it printed cannot be a crash. It is instead the delegate
# undercounting itself: `check_function_loc` and the nesting-depth rule emit inside a
# `... | while read` pipeline, so those FINDINGS_COUNT increments happen in a subshell and
# are lost. Under AUTOSPEC_COMPLEXITY_ENFORCE=1 a file that is both too long and holds one
# long function prints two blocking findings and exits 1 — an equality test called that a
# crashed gate and refused every such commit. Tracked separately; this wrapper must not
# refuse a run whose findings it is holding.
delegate_rc_matches=0
if [ "$DELEGATE_RC" -eq 0 ] && [ "$delegate_blocking" -eq 0 ]; then
  delegate_rc_matches=1
elif [ "$DELEGATE_RC" -eq 200 ] && [ "$delegate_blocking" -ge 1 ]; then
  delegate_rc_matches=1
elif [ "$DELEGATE_RC" -ge 1 ] && [ "$DELEGATE_RC" -le 64 ] \
    && [ "$delegate_blocking" -ge "$DELEGATE_RC" ]; then
  delegate_rc_matches=1
fi
if [ "$delegate_rc_matches" -ne 1 ]; then
  printf 'LINT_DELEGATE_FAILED:%s:-: exited %s without completing — gate result unknown, not clean\n' \
    "$DELEGATE" "$DELEGATE_RC"
  [ -n "$DELEGATE_OUT" ] && printf '%s\n' "$DELEGATE_OUT" >&2
  exit 2
fi

# A pure shrink: no existing file grew, something did shrink, and the change as a
# whole introduced no new material.
IS_SHRINK=0
if [ "$IS_RELOCATION" -eq 1 ] && [ "$SHRANK" -eq 1 ]; then
  IS_SHRINK=1
fi

FILTERED="$(printf '%s\n' "$DELEGATE_OUT" | awk -v shrink="$IS_SHRINK" -v safe="$NOT_GROWN" \
    -v ccok="$CC_CLEAN" -v moved="$MOVED" -v movedadds="$MOVED_ADDS" -v unitsok="$UNITS_OK" '
  BEGIN {
    n = split(safe, rows, "\n"); for (i = 1; i <= n; i++) if (rows[i] != "") ok[rows[i]] = 1
    m = split(ccok, crows, "\n"); for (i = 1; i <= m; i++) if (crows[i] != "") ccclean[crows[i]] = 1
    r = split(moved, mrows, "\n"); for (i = 1; i <= r; i++) if (mrows[i] != "") reloc[mrows[i]] = 1
    a = split(movedadds, arows, "\n"); for (i = 1; i <= a; i++) if (arows[i] != "") radd[arows[i]] = 1
  }
  # Severity is decided by the delegate — COMPLEXITY is advisory unless
  # AUTOSPEC_COMPLEXITY_ENFORCE=1 — so every rule below matches the finding text with any
  # leading INFO: stripped. Anchoring on the blocking prefix instead made these
  # suppressions silently inert under the advisory default: a file the ratchet expressly
  # waives came back as an INFO line on every commit, which is how a gate stops being read.
  { bare = $0; sub(/^INFO:/, "", bare) }
  # File-wide keyword proxy dropped when no function actually exceeds the limit.
  bare ~ /^COMPLEXITY:.*keyword-proxy cyclomatic/ {
    path = bare; sub(/^COMPLEXITY:/, "", path); sub(/:-:.*$/, "", path)
    if (path in ccclean) next
  }
  # Absolute file-LOC finding is dropped only for a file that did not get longer, and
  # downgraded to an audit-trail entry for a file created by relocating existing code.
  bare ~ /^COMPLEXITY:.*: file is [0-9]+ LOC/ {
    path = bare; sub(/^COMPLEXITY:/, "", path); sub(/:-:.*$/, "", path)
    if (path in ok) next
    if (path in reloc) {
      print "INFO:" bare " — holds relocated code, so it does not block; split it further"
      next
    }
  }
  # "file adds N lines" is a second absolute size rule, with its own hard-coded
  # threshold. A file holding relocated content adds lines by definition.
  bare ~ /^COMPLEXITY:.*: file adds [0-9]+ lines/ {
    path = bare; sub(/^COMPLEXITY:/, "", path); sub(/:-:.*$/, "", path)
    if (path in radd) {
      print "INFO:" bare " — relocated content, not new material"
      next
    }
  }
  # Per-line findings on a file that holds relocated code. Same root cause as the two
  # size rules above, in rules that judge individual lines: a new file has nothing to
  # diff against, so every one of its lines reads as freshly authored and a per-line
  # rule fires on code that merely moved. Nesting depth is the common case — moved code
  # keeps its indentation — and DOC_OUT_OF_SYNC trips on flag-shaped tokens carried
  # along inside moved test fixtures.
  #
  # Downgraded rather than dropped, so the lines stay on the record. Matches only
  # line-numbered findings; the whole-file ones are handled above. A relocation that
  # also introduces a genuinely new flag could be masked here, but that requires the
  # change to stay net-removing, and the alternative is rejecting every extraction.
  bare ~ /^(COMPLEXITY|DOC_OUT_OF_SYNC):/ {
    p = bare
    sub(/^[A-Z_]+:/, "", p)
    sub(/:[0-9]+:.*$/, "", p)
    if (p in radd) {
      print "INFO:" bare " — relocated content, not new material"
      next
    }
  }
  # Changed-line cap waived for a pure shrink; other PR_SIZE breaches still print.
  shrink == 1 && /PR_SIZE/ && /exceeded=changed_lines$/ { next }
  # Unit cap waived when the count without doc files fits. Anchored, so a combined
  # breach such as exceeded=changed_lines,logical_units still prints.
  unitsok == 1 && /PR_SIZE/ && /exceeded=logical_units$/ { next }
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
