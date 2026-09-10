#!/usr/bin/env bash
# scripts/clippy-baseline.sh — clippy warning count ratcheted against a
# committed baseline (issue #3999).
#
# The old CI step (`cargo clippy --workspace --all-targets`) always exited 0:
# warnings were printed and ignored, so a patch adding ten warnings merged
# exactly like a clean one. This gate makes the count a ratchet:
#
#   - it runs `cargo clippy --workspace --all-targets` and counts the
#     individual `warning:` lines (cargo's per-target "generated N warnings"
#     summary lines are excluded — they would double-count the same warning
#     across compilation units of one crate);
#   - it fails when the live count exceeds the count committed in
#     `config/clippy-warnings.baseline`;
#   - on failure it names every warning that is new relative to the baseline,
#     with its file, so the fix target is visible in the job log;
#   - the baseline can only be regenerated DOWNWARD: `--rebaseline` refuses
#     to record a higher count. Raising the baseline is a review rejection,
#     not a rebaseline — the debt cannot be washed away by editing the number.
#
# The baseline file stores one line per individual warning,
# `<warning text> <TAB> <file>` (sorted, duplicates preserved, comment lines
# start with `#`); the gate's count is the number of non-comment lines.
#
# Usage:
#   scripts/clippy-baseline.sh                        # live clippy, check against baseline
#   scripts/clippy-baseline.sh --rebaseline           # live clippy, rewrite baseline (downward only)
#   scripts/clippy-baseline.sh --check-file FILE      # check a saved clippy output (no build)
#   scripts/clippy-baseline.sh --rebaseline-file FILE # rewrite baseline from a saved output (downward only)
#   scripts/clippy-baseline.sh --baseline FILE        # baseline path (default config/clippy-warnings.baseline)
#   scripts/clippy-baseline.sh --help
#
# Exit codes:
#   0  pass (live count <= baseline, or baseline rewritten unchanged/lowered)
#   1  fail (count exceeds baseline, clippy errored, no baseline, or a
#      refused upward rebaseline)
#   2  usage error
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

BASELINE_FILE="$ROOT_DIR/config/clippy-warnings.baseline"
MODE="check"
SOURCE=""

usage() {
    sed -n '2,37p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --rebaseline) MODE="rebaseline" ;;
        --check-file) [ $# -ge 2 ] || { usage >&2; exit 2; }; SOURCE="$2"; shift ;;
        --rebaseline-file) MODE="rebaseline"; [ $# -ge 2 ] || { usage >&2; exit 2; }; SOURCE="$2"; shift ;;
        --baseline) [ $# -ge 2 ] || { usage >&2; exit 2; }; BASELINE_FILE="$2"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "CLIPPY_BASELINE: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

[ -z "$SOURCE" ] || [ -r "$SOURCE" ] || {
    echo "CLIPPY_BASELINE: cannot read saved clippy output: $SOURCE" >&2
    exit 1
}

# extract_signatures FILE
# One line per individual clippy warning: `<first warning line text> \t <file>`.
# A warning block is `warning: <text>` followed (after any message
# continuation lines) by `  --> <path>:<line>:<col>`. The line/column are
# dropped so an existing warning that merely moved lines is not "new".
# Cargo's per-target summaries (`warning: \`crate\` (target) generated N
# warnings...`) are not individual warnings and are skipped.
extract_signatures() {
    awk '
        function flush() {
            if (pending != "") {
                if (located) print pending "\t" path
                else         print pending "\t(nolocation)"
                pending = ""; located = 0; path = ""
            }
        }
        /^warning: `[^`]+` \([^)]*\) generated [0-9]+ warnings?/ { next }
        /^warning: / {
            flush()
            pending = substr($0, 10)
            sub(/[ \t\r]+$/, "", pending)
            located = 0
            next
        }
        pending != "" && located == 0 && /^[ \t]*-->/ {
            path = $0
            sub(/^[ \t]*-->[ \t]*/, "", path)
            sub(/[ \t\r]+$/, "", path)
            sub(/:[0-9]+:[0-9]+$/, "", path)
            located = 1
        }
        END { flush() }
    ' "$1" | LC_ALL=C sort
}

baseline_count() {
    # number of non-comment, non-blank lines in the baseline file
    grep -cvE '^(#|$)' "$1" || true
}

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
LIVE_TMP="$WORK_DIR/clippy-output"

if [ -n "$SOURCE" ]; then
    CLIPPY_OUT="$SOURCE"
else
    CLIPPY_OUT="$LIVE_TMP"
    ( cargo clippy --workspace --all-targets >"$CLIPPY_OUT" 2>&1 ) || true
    if grep -qE '^error(\[|:)' "$CLIPPY_OUT"; then
        echo "CLIPPY_BASELINE: FAIL: cargo clippy errored; the gate cannot count warnings from a failed build" >&2
        grep -E '^error(\[|:)' "$CLIPPY_OUT" | head -5 >&2 || true
        exit 1
    fi
fi

extract_signatures "$CLIPPY_OUT" > "$LIVE_TMP.sigs"
LIVE_COUNT="$(wc -l < "$LIVE_TMP.sigs" | tr -d '[:space:]')"

BASE_COUNT=0
BASELINE_EXISTS=0
if [ -f "$BASELINE_FILE" ]; then
    BASELINE_EXISTS=1
    BASE_COUNT="$(baseline_count "$BASELINE_FILE")"
fi

case "$MODE" in
check)
    if [ "$BASELINE_EXISTS" -eq 0 ]; then
        echo "CLIPPY_BASELINE: FAIL: no committed baseline at ${BASELINE_FILE#"$ROOT_DIR"/}; run 'bash scripts/clippy-baseline.sh --rebaseline' and commit it" >&2
        exit 1
    fi
    # new warnings = in live, not in baseline (multiset diff)
    { grep -vE '^(#|$)' "$BASELINE_FILE" || true; } | LC_ALL=C sort > "$LIVE_TMP.base"
    comm -13 "$LIVE_TMP.base" "$LIVE_TMP.sigs" > "$LIVE_TMP.new"
    if [ "$LIVE_COUNT" -gt "$BASE_COUNT" ]; then
        {
            echo "CLIPPY_BASELINE: FAIL: $LIVE_COUNT clippy warnings exceed the committed baseline of $BASE_COUNT (${BASELINE_FILE#"$ROOT_DIR"/})"
            while IFS=$'\t' read -r msg path; do
                [ -n "$msg" ] || continue
                echo "CLIPPY_BASELINE: new warning: $msg ($path)"
            done < "$LIVE_TMP.new"
            echo "CLIPPY_BASELINE: fix the new warnings; the baseline can only be lowered, never raised"
        } >&2
        exit 1
    fi
    echo "CLIPPY_BASELINE: OK: $LIVE_COUNT clippy warnings (baseline $BASE_COUNT, ${BASELINE_FILE#"$ROOT_DIR"/})"
    if [ -s "$LIVE_TMP.new" ]; then
        while IFS=$'\t' read -r msg path; do
            [ -n "$msg" ] || continue
            echo "CLIPPY_BASELINE: INFO: new warning present but count still under baseline: $msg ($path)"
        done < "$LIVE_TMP.new"
    fi
    ;;

rebaseline)
    if [ "$BASELINE_EXISTS" -eq 1 ] && [ "$LIVE_COUNT" -gt "$BASE_COUNT" ]; then
        echo "CLIPPY_BASELINE: FAIL: refusing to raise the baseline from $BASE_COUNT to $LIVE_COUNT (${BASELINE_FILE#"$ROOT_DIR"/})" >&2
        echo "CLIPPY_BASELINE: the baseline can only be lowered; a change that raises it is rejected in review" >&2
        exit 1
    fi
    {
        echo "# Clippy warning baseline — ratcheted gate (issue #3999)."
        echo "#"
        echo "# One line per individual \`warning:\` from"
        echo "# \`cargo clippy --workspace --all-targets\` (cargo's per-target"
        echo "# \"generated N warnings\" summaries excluded):"
        echo "#   <warning text> <TAB> <file>"
        echo "# Lines are sorted; duplicates are preserved. The gate's count is"
        echo "# the number of non-comment lines below."
        echo "#"
        echo "# Consumers(config/clippy-warnings.baseline): scripts/clippy-baseline.sh"
        echo "#"
        echo "# Regenerate only downward:"
        echo "#   bash scripts/clippy-baseline.sh --rebaseline"
        echo "# refuses to record a higher count; a change that raises this"
        echo "# baseline is a review rejection, not a rebaseline."
        echo "#"
        echo "# Carve-out tracked as a SEPARATE issue (not baselined away as"
        echo "# lint noise): the dead-code findings (\`function ... is never"
        echo "# used\`, \`struct ... is never constructed\`) are correctness"
        echo "# signals — see the follow-up issue filed from #3999."
        echo "#"
        echo "# Last rebaseline: $(date -u +%Y-%m-%d) ($LIVE_COUNT warnings)"
        cat "$LIVE_TMP.sigs"
    } > "$LIVE_TMP.newbase"
    if [ "$BASELINE_EXISTS" -eq 1 ] && cmp -s "$LIVE_TMP.newbase" "$BASELINE_FILE"; then
        echo "CLIPPY_BASELINE: baseline unchanged ($LIVE_COUNT warnings, ${BASELINE_FILE#"$ROOT_DIR"/})"
    else
        mv "$LIVE_TMP.newbase" "$BASELINE_FILE"
        echo "CLIPPY_BASELINE: baseline recorded: $LIVE_COUNT warnings (${BASELINE_FILE#"$ROOT_DIR"/})"
    fi
    ;;
esac
