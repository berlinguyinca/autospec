#!/usr/bin/env bash
# scripts/test-failures-baseline.sh — known-failing-test ratchet (issue #4291).
#
# Two tests in `cargo test -p autospec-cli --test autonomous_conductor_commands`
# fail on main, so every conversion gate that runs the suite carries a permanent
# floor of 2 tolerated failures. A floor is not a baseline: with no record of
# WHICH failures are known, "2 failed" is indistinguishable from "2 known ones
# plus a new one nobody noticed", and a gate cannot compare against a number it
# never wrote down. That is the difference between the conversion-gate statuses
# `NEW-TEST-FAILURES` (a baseline exists and attributes the failures) and
# `UNKNOWN-NO-BASELINE` (failures with nothing to attribute them to — a harness
# fault, not a property of the patch; see docs/conversion-gate.md).
#
# This script makes the known-failing set a file, one entry per known-failing
# test, so the set has one source of truth instead of an observation restated
# (and driftingly restated) inside each consumer:
#
#   autospec/baseline-failures.txt
#
# Entry format (tab-separated, sorted with LC_ALL=C sort, no duplicate ids):
#
#   <cargo test id> <TAB> issue=#<N> [<TAB> binary=<cargo test target>]
#
# Every entry must carry an `issue=#<N>` column. A known failure with no linked
# issue is a failure nobody owns: nothing schedules its removal, and the entry
# silently keeps tolerating the test after it is fixed, masking the next
# regression of it. Growth of the baseline is therefore refused in two places:
#
#   * --rebaseline only lowers the baseline. An id that is failing but not
#     already listed is refused by name; the script will not invent the linkage.
#   * Adding an entry by hand requires the issue reference, and both check and
#     rebaseline lint for it (BASELINE_ENTRY_NO_ISSUE).
#
# An entry whose test now passes is a hard finding (STALE_BASELINE_ENTRY), not
# a note: a baselined id tolerates whatever fails under that name, so a fixed
# test left in the file is a hole reopened the moment it regresses again.
#
# Usage:
#   scripts/test-failures-baseline.sh [MODE] [OPTIONS]
#
# Modes:
#   --check                (default) run the targets, compare failures to the
#                          baseline. Fails on any failure not baselined, on a
#                          harness that never ran, and on a stale entry.
#   --lint-baseline-only   validate baseline syntax only (no cargo): every
#                          entry parses, carries issue=#<N>, ids sorted, unique.
#   --rebaseline           recompute the baseline from a run and rewrite it
#                          DOWNWARD only: entries whose test demonstrably
#                          passes now are dropped, nothing is ever added.
#
# Options:
#   --package NAME         cargo package to test (default: autospec-cli)
#   --test NAME            cargo test target; repeatable
#                          (default: autonomous_conductor_commands)
#   --run-log FILE         check a captured cargo test log instead of running
#                          cargo (the log must contain cargo's stdout+stderr)
#   --baseline FILE        baseline path (default: $AUTOSPEC_TEST_FAILURE_BASELINE,
#                          else autospec/baseline-failures.txt at the repo root)
#   -h, --help             show this header
#
# Exit codes:
#   0  live failures are exactly the baselined ones (stale entries and
#      non-executed entries are reported as findings and warnings)
#   1  at least one blocking finding
#   2  usage or environment error (unknown flag, missing baseline, no cargo)
#
# Related: scripts/clippy-baseline.sh (issue #3999) is the same ratchet shape for
# clippy warnings. The baseline is never generated from a run alone, and the
# baseline file does not feed a mtime-gated build, so it is not a
# generated-artifact with consumers to pin.

set -uo pipefail

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

DEFAULT_TARGET="autonomous_conductor_commands"
DEFAULT_PACKAGE="autospec-cli"

BASELINE_FILE="${AUTOSPEC_TEST_FAILURE_BASELINE:-$ROOT_DIR/autospec/baseline-failures.txt}"
PACKAGE="$DEFAULT_PACKAGE"
TARGETS=()
RUN_LOG=""
MODE="check"

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die() {
    printf 'ERROR: %s\n' "$2" >&2
    exit "$1"
}

need_value() {
    # need_value <flag> <remaining-argc>
    [ "$2" -ge 2 ] || die 2 "option $1 requires a value"
}

relpath() {
    local p="$1"
    case "$p" in
        "$ROOT_DIR"/*) printf '%s\n' "${p#"$ROOT_DIR"/}" ;;
        *) printf '%s\n' "$p" ;;
    esac
}

# ---- argument parsing -------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --check) MODE="check" ;;
        --rebaseline) MODE="rebaseline" ;;
        --lint-baseline-only) MODE="lint" ;;
        --run-log) need_value "$1" $#; RUN_LOG="$2"; shift ;;
        --package) need_value "$1" $#; PACKAGE="$2"; shift ;;
        --test) need_value "$1" $#; TARGETS+=("$2"); shift ;;
        --baseline) need_value "$1" $#; BASELINE_FILE="$2"; shift ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'ERROR: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

[ ${#TARGETS[@]} -gt 0 ] || TARGETS=("$DEFAULT_TARGET")

BASELINE_REL=$(relpath "$BASELINE_FILE")

# ---- baseline lint ----------------------------------------------------------

# Prints one finding line per problem in the baseline file. No findings = clean.
lint_baseline() {
    local file="$1"
    local norm="$WORK_DIR/lint-normalized"

    awk 'BEGIN { FS = "\t" }
    /^[ \t]*$/ || /^#/ { next }
    {
        if (NF < 2) {
            printf "BASELINE_ENTRY_MALFORMED: %s:%d: `%s` is not `<test id>\\tissue=#<N>` (tab-separated)\n", FILELABEL, NR, $1
            next
        }
        if ($2 !~ /^issue=#[1-9][0-9]*$/) {
            printf "BASELINE_ENTRY_NO_ISSUE: %s:%d: `%s` has no `issue=#<N>` column: a known failure without a linked issue is unowned and never scheduled for removal\n", FILELABEL, NR, $1
            next
        }
        if (NF > 3) {
            printf "BASELINE_ENTRY_MALFORMED: %s:%d: `%s` has more than 3 tab-separated columns\n", FILELABEL, NR, $1
            next
        }
        if (NF == 3 && $3 !~ /^binary=[^ \t]+$/) {
            printf "BASELINE_ENTRY_MALFORMED: %s:%d: third column of `%s` must be `binary=<cargo test target>`\n", FILELABEL, NR, $1
            next
        }
    }' FILELABEL="$BASELINE_REL" "$file"

    awk 'BEGIN { FS = "\t" } /^[ \t]*$/ || /^#/ { next } { print }' "$file" >"$norm"

    local dups
    dups=$(cut -f1 "$norm" | LC_ALL=C sort | uniq -d)
    if [ -n "$dups" ]; then
        printf '%s\n' "$dups" | while IFS= read -r d; do
            printf 'BASELINE_DUPLICATE_ID: %s: `%s` appears more than once\n' "$BASELINE_REL" "$d"
        done
    fi

    if ! LC_ALL=C sort -c "$norm" >/dev/null 2>&1; then
        printf 'BASELINE_UNSORTED: %s: entries are not sorted with LC_ALL=C sort\n' "$BASELINE_REL"
    fi
}

# ---- run-log parsing --------------------------------------------------------

baseline_entry_ids() {
    awk 'BEGIN { FS = "\t" } /^[ \t]*$/ || /^#/ { next } { print $1 }' "$1" | LC_ALL=C sort -u
}

# Live failures with the test target they came from: `<id>\t<target>`.
live_failures() {
    awk '{ line = $0; sub(/\r$/, "", line) }
    line ~ /^ *Running (unittests |)(tests\/|src\/|[A-Za-z0-9._/-]+\.rs)/ {
        t = line
        sub(/^ *Running (unittests |)/, "", t)
        sub(/^tests\//, "", t)
        sub(/\.rs.*/, "", t)
        target = t
        next
    }
    line ~ /^test .* \.\.\. FAILED[ \t]*$/ {
        id = line
        sub(/^test /, "", id)
        sub(/[ \t]*\.\.\. FAILED[ \t]*$/, "", id)
        if (id != "") printf "%s\t%s\n", id, target
    }' "$1" | LC_ALL=C sort -u
}

live_passes() {
    awk '{ line = $0; sub(/\r$/, "", line) }
    line ~ /^test .* \.\.\. ok[ \t]*$/ {
        id = line
        sub(/^test /, "", id)
        sub(/[ \t]*\.\.\. ok[ \t]*$/, "", id)
        if (id != "") print id
    }' "$1" | LC_ALL=C sort -u
}

declared_failed_total() {
    awk '/^test result: / {
        if (match($0, /[0-9]+ failed/)) total += substr($0, RSTART, RLENGTH) + 0
    }
    END { print total + 0 }' "$1"
}

harness_ran() {
    grep -q '^test result: ' "$1"
}

compile_error_present() {
    grep -qE '^error(\[E[0-9]+\])?: |^error: could not compile' "$1"
}
# ---- shared harness verification -------------------------------------------

# verify_log LOG -> 0 if the log is a usable measurement, 1 if it is not
# (findings already printed).
verify_log() {
    local log="$1" bad=0

    if ! harness_ran "$log"; then
        if compile_error_present "$log"; then
            printf 'TESTS_DO_NOT_COMPILE: the test targets failed to build; no baseline can attribute a test that does not compile\n'
        else
            printf 'HARNESS_NEVER_RAN: no `test result:` line in the cargo output; this is not a pass, it is a harness that did not run (--test NAME must name a test target, not a support module)\n'
        fi
        bad=1
    fi

    local declared parsed
    declared=$(declared_failed_total "$log")
    parsed=$(count_lines "$WORK_DIR/live_ids")
    if [ "$declared" -gt 0 ] && [ "$parsed" -eq 0 ]; then
        printf 'FAILURE_ATTRIBUTION: the run reports %s failed test(s) but no `test ... FAILED` line was parsed; the gate cannot attribute the failures, so nothing is tolerated\n' "$declared"
        bad=1
    elif [ "$declared" != "$parsed" ]; then
        printf 'WARN: run declares %s failed test(s), %s distinct FAILED test ids parsed (a test may be listed twice across targets)\n' "$declared" "$parsed"
    fi

    return "$bad"
}

# ---- shared mode plumbing ---------------------------------------------------

# obtain_log WORKFILE -> 0; fills WORKFILE with the cargo output to measure,
# either copied from --run-log or from a fresh `cargo test` run.
obtain_log() {
    local log="$1"
    if [ -n "$RUN_LOG" ]; then
        [ -f "$RUN_LOG" ] || die 2 "run log not found: $RUN_LOG"
        cp "$RUN_LOG" "$log"
        return 0
    fi
    command -v cargo >/dev/null 2>&1 ||
        die 2 "cargo not found on PATH; pass --run-log FILE to check a captured run"
    local cmd=(cargo test -p "$PACKAGE")
    local t
    for t in "${TARGETS[@]}"; do cmd+=(--test "$t"); done
    printf 'INFO: running: %s\n' "${cmd[*]}"
    ( "${cmd[@]}" ) >"$log" 2>&1 || true
}

# parse_run_sets LOG -> 0; derives the id sets the modes compare.
parse_run_sets() {
    local log="$1"
    live_failures "$log" >"$WORK_DIR/live_full"
    cut -f1 "$WORK_DIR/live_full" | LC_ALL=C sort -u >"$WORK_DIR/live_ids"
    live_passes "$log" >"$WORK_DIR/ok_ids"
    baseline_entry_ids "$BASELINE_FILE" >"$WORK_DIR/base_ids"
}

count_lines() {
    wc -l <"$1" | tr -d ' '
}

# report_new_failures -> prints one NEW_TEST_FAILURE line per live failure that
# is not baselined; sets NEW_COUNT to the number printed.
report_new_failures() {
    local new_out
    new_out=$(awk -F'\t' 'NR == FNR { base[$0] = 1; next } !($1 in base)' \
        "$WORK_DIR/base_ids" "$WORK_DIR/live_full")
    NEW_COUNT=0
    [ -n "$new_out" ] || return 0
    NEW_COUNT=$(count_lines_text "$new_out")
    printf '%s\n' "$new_out" | while IFS=$'\t' read -r id target; do
        printf 'NEW_TEST_FAILURE: `%s` is failing and is not in %s' "$id" "$BASELINE_REL"
        [ -n "$target" ] && printf ' (target=%s)' "$target"
        printf '; fix it, or hand-add `<id>\\tissue=#<N>` after opening an issue\n'
    done
}

# report_stale_and_unrun -> a baselined id that now passes is blocking
# (STALE_BASELINE_ENTRY); one that was merely not executed is a note. Sets
# STALE_COUNT to the number of blocking ids.
report_stale_and_unrun() {
    local stale unrun
    stale=$(comm -12 "$WORK_DIR/base_ids" "$WORK_DIR/ok_ids")
    unrun=$(comm -23 "$WORK_DIR/base_ids" "$WORK_DIR/live_ids" |
        comm -23 - "$WORK_DIR/ok_ids")
    STALE_COUNT=0
    if [ -n "$stale" ]; then
        STALE_COUNT=$(count_lines_text "$stale")
        printf '%s\n' "$stale" | while IFS= read -r id; do
            printf 'STALE_BASELINE_ENTRY: `%s` passes but is still in %s; a baselined id tolerates whatever fails under that name, so remove the line (`--rebaseline` drops it)\n' "$id" "$BASELINE_REL"
        done
    fi
    if [ -n "$unrun" ]; then
        printf '%s\n' "$unrun" | while IFS= read -r id; do
            printf 'WARN: baseline entry `%s` was neither run nor failed in this run (filtered out?); not treated as fixed\n' "$id"
        done
    fi
}

# rewrite_baseline -> writes $WORK_DIR/baseline.new from the current baseline,
# dropping only entries this run proved passing (an entry that was not executed
# stays: silence is not evidence of a fix). Sets AFTER_COUNT.
rewrite_baseline() {
    local new_file="$WORK_DIR/baseline.new"
    awk 'BEGIN { FS = "\t" }
    NR == FNR { fixed[$0] = 1; next }
    /^[ \t]*#/ || /^[ \t]*$/ { print; next }
    { if (!($1 in fixed)) print }' "$WORK_DIR/ok_ids" "$BASELINE_FILE" >"$new_file"

    local recheck
    recheck=$(awk 'BEGIN { FS = "\t" } /^[ \t]*$/ || /^#/ { next } { print }' "$new_file" |
        LC_ALL=C sort -c 2>&1)
    [ -z "$recheck" ] || die 1 "internal error: rewritten baseline is not sorted; original left in place"

    AFTER_COUNT=$(awk 'BEGIN { FS = "\t" } /^[ \t]*$/ || /^#/ { next } { n++ } END { print n + 0 }' "$new_file")
}

count_lines_text() {
    printf '%s\n' "$1" | wc -l | tr -d ' '
}

# ---- modes ------------------------------------------------------------------

FINDINGS=0

do_lint() {
    [ -f "$BASELINE_FILE" ] || die 2 "baseline file not found: $BASELINE_REL (create it with one line per known-failing test: '<test id>\\tissue=#<N>')"
    local out
    out=$(lint_baseline "$BASELINE_FILE")
    if [ -n "$out" ]; then
        printf '%s\n' "$out"
        FINDINGS=$((FINDINGS + $(printf '%s\n' "$out" | wc -l | tr -d ' ')))
        return 1
    fi
    local n
    n=$(baseline_entry_ids "$BASELINE_FILE" | wc -l | tr -d ' ')
    printf 'BASELINE_OK: %s: %s known-failing test(s), all with issue references\n' "$BASELINE_REL" "$n"
    return 0
}

do_check() {
    [ -f "$BASELINE_FILE" ] || die 2 "baseline file not found: $BASELINE_REL (create it with one line per known-failing test: '<test id>\\tissue=#<N>')"

    local lint_out
    lint_out=$(lint_baseline "$BASELINE_FILE")
    if [ -n "$lint_out" ]; then
        printf '%s\n' "$lint_out"
        FINDINGS=$((FINDINGS + $(count_lines_text "$lint_out")))
    fi

    local log="$WORK_DIR/run.log"
    obtain_log "$log"
    parse_run_sets "$log"

    if ! verify_log "$log"; then
        FINDINGS=$((FINDINGS + 1))
    fi

    report_new_failures
    FINDINGS=$((FINDINGS + NEW_COUNT))
    report_stale_and_unrun
    FINDINGS=$((FINDINGS + STALE_COUNT))

    local n_live n_base
    n_live=$(count_lines "$WORK_DIR/live_ids")
    n_base=$(count_lines "$WORK_DIR/base_ids")
    if [ "$FINDINGS" -eq 0 ]; then
        printf 'BASELINE_MATCH: %s failing test(s), all present in %s (%s baselined)\n' "$n_live" "$BASELINE_REL" "$n_base"
    else
        printf 'FAIL: %s blocking finding(s) (live failures=%s, baseline entries=%s)\n' "$FINDINGS" "$n_live" "$n_base"
    fi
    [ "$FINDINGS" -eq 0 ]
}

do_rebaseline() {
    [ -f "$BASELINE_FILE" ] || die 2 "baseline file not found: $BASELINE_REL; --rebaseline only lowers an existing baseline and never creates one (a new entry needs a hand-written issue reference)"

    local lint_out
    lint_out=$(lint_baseline "$BASELINE_FILE")
    if [ -n "$lint_out" ]; then
        printf '%s\n' "$lint_out" >&2
        die 1 "refusing to rewrite a baseline that does not lint clean: $BASELINE_REL"
    fi

    local log="$WORK_DIR/run.log"
    obtain_log "$log"
    parse_run_sets "$log"

    if ! verify_log "$log"; then
        die 1 "refusing to rebaseline from a run that is not a usable measurement (see findings above)"
    fi

    # Downward only: a live failure that is not already listed would need an
    # issue reference this script cannot invent.
    local additions
    additions=$(comm -13 "$WORK_DIR/base_ids" "$WORK_DIR/live_ids")
    if [ -n "$additions" ]; then
        printf '%s\n' "$additions" | while IFS= read -r id; do
            printf 'REBASELINE_REFUSED_UPWARD: `%s` is failing but is not in %s\n' "$id" "$BASELINE_REL"
        done
        printf 'REBASELINE_REFUSED: --rebaseline only lowers the baseline; open an issue per new failure and hand-add `<id>\\tissue=#<N>`\n' >&2
        exit 1
    fi

    # Drop only entries proven to pass now; an entry that was not executed
    # stays (silence is not evidence of a fix).
    rewrite_baseline

    local before
    before=$(count_lines "$WORK_DIR/base_ids")
    if cmp -s "$WORK_DIR/baseline.new" "$BASELINE_FILE"; then
        printf 'BASELINE_UNCHANGED: %s still has %s known-failing test(s)\n' "$BASELINE_REL" "$AFTER_COUNT"
        exit 0
    fi
    cp "$WORK_DIR/baseline.new" "$BASELINE_FILE" || die 1 "could not write $BASELINE_REL"
    printf 'BASELINE_SHRUNK: %s: %s -> %s known-failing test(s); %s dropped as passing\n' \
        "$BASELINE_REL" "$before" "$AFTER_COUNT" "$((before - AFTER_COUNT))"
}

case "$MODE" in
    lint) do_lint ;;
    check) do_check ;;
    rebaseline) do_rebaseline ;;
    *) die 2 "unknown mode: $MODE" ;;
esac
