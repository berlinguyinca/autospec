#!/usr/bin/env sh
# scripts/verify-gate.sh — run declared verification lanes where a lane that
# measured nothing is reported `unknown`, never as a pass.
#
# Why this exists: gates on this repo have answered "did the checks find
# anything?" with "is the output non-empty?" — `dogfood-adapter-doc-drift.sh`
# still says "If the gate produced no JSON (e.g. errored out) leave VERDICT_FILE
# alone", and `explore-qa-gate.sh` writes a non-failure for a verdict file it
# never received. One of them read `$?` off the tail of a pipeline. Each said
# PASS for a run in which nothing was verified: a linter absent from PATH, a
# test binary that died before printing its summary, a suite whose reporter
# format changed under the parser. `ui-evidence-gates.sh` fixed this for the
# runtime UI tier (exit 2 = UNKNOWN, "an absent browser is not a passing
# grade"); this is the same rule for any command-declared lane, in POSIX sh so
# it runs on a busybox host as well as on this one.
#
# Three rules, and the script exists to hold them:
#
#   1. Assert the toolchain BEFORE measuring. A missing executable is reported
#      by name as UNAVAILABLE before a single lane runs, because a linter that
#      is not installed produced zero findings *and* zero evidence.
#   2. Take the exit status from the command, never from its output. Every lane
#      runs as `sh -c '<command>' >file 2>&1` with the status captured
#      directly. No pipe is introduced, so there is no tail-of-pipeline status
#      to mistake for the head's, and an empty output file is never success.
#   3. A lane that produced no parseable result line measured nothing, and
#      nothing measured is `unknown` — not zero failures. `0` is a count you can
#      trust; `unknown` says the counter never ran.
#
# Every numeric field in the status record therefore carries the literal string
# `unknown` when it was never measured, which is a different value from `0` in
# the JSON, on the status line, and in the exit code.
#
# Exit codes (mirroring ui-evidence-gates.sh, plus 3 for the preflight):
#
#   0  PASS         every lane ran, parsed, and found nothing
#   1  FAIL         at least one lane failed
#   2  UNKNOWN      at least one lane measured nothing, and none failed
#   3  UNAVAILABLE  a required tool is not on PATH; nothing was measured
#   64              usage error
#
# Findings outrank unknown: a lane that could not run must not mask a defect a
# lane that did run actually reported.

set -eu

usage() {
    cat <<'EOF'
Usage:
  verify-gate.sh [--repo-root <dir>] [--report <file>] [--require-tool <tool>]...
                 [--fail-regex <ERE>] <manifest>

Manifest: one lane per line, tab-separated:

  <name>\t<command>[\t<result-regex>]

  <name>          [A-Za-z0-9._-]+, used as the record key
  <command>       run by `sh -c` from --repo-root, stdout and stderr captured to
                  a file rather than piped
  <result-regex>  ERE marking a line the lane counted as a result. Omit it and
                  the lane can never prove it measured anything, so it is
                  reported unknown rather than passed.

A result line reporting only zero failures — '5 passed; 0 failed' — is not
counted as a failure line; a line mixing a zero and a non-zero count is.

Blank lines and lines starting with '#' are ignored.

Status record (one per lane, plus the aggregate):

  {"lane":"<name>","status":"pass|fail|unknown","exit_code":<int>|"unknown",
   "result_lines":<int>|"unknown","failed_lines":<int>|"unknown"}

Final stdout line (parse this, not the exit code alone):

  verify-gate: PASS|FAIL|UNKNOWN|UNAVAILABLE (<n> lanes, <n> failed, <n> unknown)
EOF
}

die_usage() {
    printf 'verify-gate: %s\n' "$*" >&2
    usage >&2
    exit 64
}

# json_number_or_unknown <value>
#
# A measured count is emitted as a JSON number; the token `unknown` is emitted
# as the JSON string "unknown". Never as 0 — collapsing the two is the defect
# this script was written against.
json_number_or_unknown() {
    case "$1" in
        '' | unknown | *[!0-9]*) printf '"unknown"' ;;
        *) printf '%s' "$1" ;;
    esac
}

# as_count <value> — a non-negative integer passes through unchanged; anything
# else (a warning line, a usage string, empty output) is the token `unknown`.
as_count() {
    case "$1" in
        '' | *[!0-9]*) printf 'unknown' ;;
        *) printf '%s' "$1" ;;
    esac
}

# json_string_array <words...>
json_string_array() {
    printf '['
    first=1
    for item in "$@"; do
        [ "$first" -eq 1 ] || printf ','
        first=0
        printf '"%s"' "$item"
    done
    printf ']'
}

REPO_ROOT="$(pwd)"
REPORT=""
MANIFEST=""
REQUIRED_TOOLS=""
FAIL_REGEX='FAIL|fail|ERROR|error'

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root)
            [ "$#" -ge 2 ] || die_usage "--repo-root requires a value"
            REPO_ROOT="$2"
            shift 2
            ;;
        --report)
            [ "$#" -ge 2 ] || die_usage "--report requires a value"
            REPORT="$2"
            shift 2
            ;;
        --require-tool)
            [ "$#" -ge 2 ] || die_usage "--require-tool requires a value"
            REQUIRED_TOOLS="$REQUIRED_TOOLS $2"
            shift 2
            ;;
        --fail-regex)
            [ "$#" -ge 2 ] || die_usage "--fail-regex requires a value"
            FAIL_REGEX="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --*) die_usage "unknown argument: $1" ;;
        *)
            [ -z "$MANIFEST" ] || die_usage "more than one manifest given"
            MANIFEST="$1"
            shift
            ;;
    esac
done

[ -n "$MANIFEST" ] || die_usage "a manifest is required"
[ -f "$MANIFEST" ] || die_usage "manifest does not exist: $MANIFEST"
[ -d "$REPO_ROOT" ] || die_usage "--repo-root does not exist: $REPO_ROOT"
cd "$REPO_ROOT"

write_report() {
    [ -n "$REPORT" ] || return 0
    # Only strip a directory when there is one: `${REPORT%/*}` on a bare
    # 'report.json' leaves the name itself, and mkdir -p would create a
    # directory where the report belongs.
    case "$REPORT" in
        */*) mkdir -p "${REPORT%/*}" 2>/dev/null || true ;;
    esac
    cp "$WORK/report.json" "$REPORT"
}

# ---------------------------------------------------------------------------
# Rule 1 — the toolchain assertion, before anything is measured.
#
# The gate's own parser is part of the toolchain: a run without awk cannot
# count result lines, so awk is required whether or not the caller declared it.
# Declared tools are checked in the same pass, before a lane is dispatched, so
# a missing linter never reaches a point where its silence could be read as a
# result.
# ---------------------------------------------------------------------------
# awk counts the result lines, mktemp makes the per-lane capture files, cat
# reads them back: a run missing any of them has measured nothing.
INTERNAL_TOOLS='awk cat mktemp'

MISSING_COUNT=0
MISSING_LIST=""
# shellcheck disable=SC2086  # word splitting of the tool lists is the point
for tool in $INTERNAL_TOOLS $REQUIRED_TOOLS; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        MISSING_COUNT=$((MISSING_COUNT + 1))
        MISSING_LIST="$MISSING_LIST $tool"
    fi
done

if [ "$MISSING_COUNT" -gt 0 ]; then
    # shellcheck disable=SC2086  # word splitting of the list is the point
    for tool in $MISSING_LIST; do
        printf 'verify-gate: UNAVAILABLE — missing tool: %s\n' "$tool" >&2
    done
    # Nothing ran, so nothing has a number. Record `unknown` rather than the
    # zero an uninitialised counter would otherwise have written. Written
    # straight to --report because $WORK does not exist yet — creating it would
    # need the very tool that may be the one missing.
    # shellcheck disable=SC2086  # word splitting of the list is the point
    missing_json="$(json_string_array $MISSING_LIST)"
    if [ -n "$REPORT" ]; then
        case "$REPORT" in
            */*) mkdir -p "${REPORT%/*}" 2>/dev/null || true ;;
        esac
        printf '{"schema":1,"status":"UNAVAILABLE","rule":"tool-unavailable","missing_tools":%s,"total":"unknown","passed":"unknown","failed":"unknown","unknown":"unknown","lanes":[]}\n' \
            "$missing_json" >"$REPORT"
    fi
    printf 'verify-gate: UNAVAILABLE (0 lanes, 0 failed, 0 unknown)\n'
    exit 3
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# count_matches <file> <result-regex> <fail-regex> <which>
#
# Patterns travel through the environment rather than `awk -v`, because -v
# processes backslash escapes in the value and would silently rewrite a regex
# such as 'not\ ok'.
#
# A result line is only a failure when it actually reports a non-zero count.
# The ubiquitous test-summary line — `test result: ok. 5 passed; 0 failed` —
# contains the word "failed" while reporting the opposite, and a gate that
# reads it as a defect is a gate that gets silenced within a week. The guard is
# narrow: it suppresses only when every failure-ish token on the line is
# preceded by a zero, so `0 failed but 1 error` still counts as a failure.
count_matches() {
    LANE_COUNT_FILE="$1"
    LANE_RESULT_RE="$2"
    LANE_FAIL_RE="$3"
    LANE_COUNT_WHICH="$4"
    export LANE_RESULT_RE LANE_FAIL_RE LANE_COUNT_WHICH
    awk '
        BEGIN {
            re = ENVIRON["LANE_RESULT_RE"]
            fre = ENVIRON["LANE_FAIL_RE"]
            which = ENVIRON["LANE_COUNT_WHICH"]
            total = 0
            bad = 0
        }
        function zero_only_failures(line,    l) {
            l = tolower(line) " "
            if (l !~ /failed|failure|failures|error|errors/) { return 0 }
            if (l ~ /(^|[^0-9])[1-9][0-9]*[ \t]*(failed|failure|failures|error|errors)([^0-9]|$)/) {
                return 0
            }
            if (l ~ /(failed|failure|failures|error|errors)[ \t]*[:=][ \t]*0([^0-9]|$)/) { return 1 }
            return (l ~ /(^|[^0-9])0[ \t]*(failed|failure|failures|error|errors)([^0-9]|$)/)
        }
        re != "" && $0 ~ re {
            total++
            if ((fre == "" || $0 ~ fre) && !zero_only_failures($0)) { bad++ }
        }
        END { printf "%d\n", (which == "total" ? total : bad) }
    ' "$LANE_COUNT_FILE"
}

total=0
passed=0
failed=0
unknown=0

run_lane() {
    lane_name="$1"
    lane_command="$2"
    lane_result_re="$3"

    total=$((total + 1))

    if [ -z "$lane_command" ]; then
        record_lane "$lane_name" unknown unknown unknown unknown
        printf 'verify-gate: %s: UNKNOWN — the lane declares no command\n' "$lane_name" >&2
        return
    fi

    # Rule 2 — the status comes from the command itself. There is no pipe here
    # on purpose: `cmd | tail` reports the status of `tail`, which is 0 even
    # when the command under test died.
    lane_code=0
    sh -c "$lane_command" >"$WORK/$lane_name.out" 2>&1 || lane_code=$?

    if [ "$lane_code" -eq 127 ]; then
        # 127 is the shell saying it could not start the program at all. That
        # lane never ran; it is not a lane that found a defect, and it is
        # certainly not a lane that found nothing. The status is unknown but the
        # exit code is recorded as observed — "capture the real exit status" is
        # not conditional on the status being interesting.
        record_lane "$lane_name" unknown "$lane_code" unknown unknown
        printf 'verify-gate: %s: UNKNOWN — command not found\n' "$lane_name" >&2
        return
    fi

    # A non-zero exit outranks everything downstream of it: the command said it
    # failed, and that is the answer whether or not its output parsed. Checking
    # the exit code first is what keeps "empty output, exit 3" from being read
    # as "nothing measured, therefore not a failure".
    if [ "$lane_code" -ne 0 ]; then
        record_failed_lane "$lane_name" "$lane_code" "$lane_result_re"
        return
    fi

    record_measured_lane "$lane_name" "$lane_code" "$lane_result_re"
}

# record_failed_lane <name> <exit-code> <result-regex>
#
# The lane said it failed. Its output is still counted, because "3 of 40 result
# lines were failures" is the useful record even when the command exited 1.
record_failed_lane() {
    if [ -n "$3" ]; then
        record_lane "$1" fail "$2" \
            "$(count_matches "$WORK/$1.out" "$3" "$FAIL_REGEX" total)" \
            "$(count_matches "$WORK/$1.out" "$3" "$FAIL_REGEX" bad)"
    else
        record_lane "$1" fail "$2" unknown unknown
    fi
}

# record_measured_lane <name> <exit-code> <result-regex>
#
# The lane exited 0, so its output is the only evidence left — and output that
# yields nothing to count is itself a verdict, `unknown`, not a pass.
record_measured_lane() {
    lane_name="$1"
    lane_code="$2"
    lane_result_re="$3"

    if [ -z "$lane_result_re" ]; then
        # Exit 0 with no result pattern means nothing distinguishes "the suite
        # ran and passed" from "the suite printed nothing". Fail closed.
        record_lane "$lane_name" unknown "$lane_code" unknown unknown
        printf 'verify-gate: %s: UNKNOWN — no result regex, output never measured\n' "$lane_name" >&2
        return
    fi

    lane_results="$(as_count "$(count_matches "$WORK/$lane_name.out" "$lane_result_re" "$FAIL_REGEX" total)")"
    lane_failed="$(as_count "$(count_matches "$WORK/$lane_name.out" "$lane_result_re" "$FAIL_REGEX" bad)")"

    if [ "$lane_results" = 'unknown' ]; then
        record_lane "$lane_name" unknown "$lane_code" unknown unknown
        printf 'verify-gate: %s: UNKNOWN — result count unreadable\n' "$lane_name" >&2
        return
    fi

    if [ "$lane_results" -eq 0 ]; then
        # Rule 3 — exit 0 with an output that parsed to nothing. This is the
        # shape of a suite that died quietly, or of a reporter whose format
        # changed. It is not zero failures.
        record_lane "$lane_name" unknown "$lane_code" 0 0
        printf 'verify-gate: %s: UNKNOWN — exit 0, no result line parsed\n' "$lane_name" >&2
        return
    fi

    if [ "$lane_failed" = 'unknown' ]; then
        # Results were counted but the failure count is not readable, so the
        # lane is not known-clean either.
        record_lane "$lane_name" unknown "$lane_code" "$lane_results" unknown
        printf 'verify-gate: %s: UNKNOWN — failed-line count unreadable\n' "$lane_name" >&2
        return
    fi

    if [ "$lane_failed" -gt 0 ]; then
        record_lane "$lane_name" fail "$lane_code" "$lane_results" "$lane_failed"
        return
    fi

    record_lane "$lane_name" pass "$lane_code" "$lane_results" "$lane_failed"
}

# record_lane <name> <status> <exit> <result_lines> <failed_lines>
#
# `unknown` in the exit slot means the lane never ran. The counters are only
# touched here, so a lane cannot be counted twice or land in no bucket.
record_lane() {
    name="$1"
    printf '%s' "$2" >"$WORK/$name.status"
    printf '%s' "$3" >"$WORK/$name.exit"
    printf '%s' "$4" >"$WORK/$name.results"
    printf '%s' "$5" >"$WORK/$name.failed"

    case "$2" in
        pass) passed=$((passed + 1)) ;;
        fail) failed=$((failed + 1)) ;;
        *) unknown=$((unknown + 1)) ;;
    esac
}

parse_manifest() {
    # Emit the three manifest columns for every real lane line, NUL-safe on the
    # command column because the separator is a tab and read takes the rest of
    # the line into the final variable.
    while IFS="$(printf '\t')" read -r lane_name lane_command lane_result_re; do
        case "$lane_name" in
            '' | '#'*) continue ;;
        esac
        # Strip a trailing carriage return so a CRLF manifest cannot turn the
        # result regex into one that nothing matches.
        lane_name="${lane_name%"$(printf '\r')"}"
        lane_command="${lane_command%"$(printf '\r')"}"
        lane_result_re="${lane_result_re%"$(printf '\r')"}"

        case "$lane_name" in
            *[!A-Za-z0-9._-]*)
                die_usage "lane name must match [A-Za-z0-9._-]+: $lane_name"
                ;;
        esac

        printf '%s\t%s\t%s\n' "$lane_name" "$lane_command" "$lane_result_re"
    done
}

parse_manifest <"$MANIFEST" >"$WORK/lanes.tsv"

while IFS="$(printf '\t')" read -r lane_name lane_command lane_result_re; do
    [ -n "$lane_name" ] || continue
    run_lane "$lane_name" "$lane_command" "$lane_result_re"
done <"$WORK/lanes.tsv"

if [ "$total" -eq 0 ]; then
    # An empty manifest verified nothing, and a gate with nothing to say is not
    # a gate that passed.
    printf '{"schema":1,"status":"UNKNOWN","rule":"empty-manifest","total":0,"passed":0,"failed":0,"unknown":"unknown","lanes":[]}\n' \
        >"$WORK/report.json"
    write_report
    printf 'verify-gate: UNKNOWN (0 lanes, 0 failed, 0 unknown)\n'
    exit 2
fi

if [ "$failed" -gt 0 ]; then
    overall='FAIL'
    exit_code=1
elif [ "$unknown" -gt 0 ]; then
    overall='UNKNOWN'
    exit_code=2
else
    overall='PASS'
    exit_code=0
fi

{
    printf '{"schema":1,"status":"%s","total":%d,"passed":%d,"failed":%d,"unknown":%d,"lanes":[' \
        "$overall" "$total" "$passed" "$failed" "$unknown"
    first=1
    while IFS="$(printf '\t')" read -r lane_name lane_command lane_result_re; do
        [ -n "$lane_name" ] || continue
        [ -f "$WORK/$lane_name.status" ] || continue
        [ "$first" -eq 1 ] || printf ','
        first=0
        printf '{"lane":"%s","status":"%s","exit_code":%s,"result_lines":%s,"failed_lines":%s}' \
            "$lane_name" "$(cat "$WORK/$lane_name.status")" \
            "$(json_number_or_unknown "$(cat "$WORK/$lane_name.exit")")" \
            "$(json_number_or_unknown "$(cat "$WORK/$lane_name.results")")" \
            "$(json_number_or_unknown "$(cat "$WORK/$lane_name.failed")")"
    done <"$WORK/lanes.tsv"
    printf ']}\n'
} >"$WORK/report.json"

write_report

# Last line on purpose, so a caller can `tail -1` rather than parse the run.
printf 'verify-gate: %s (%d lanes, %d failed, %d unknown)\n' \
    "$overall" "$total" "$failed" "$unknown"
exit "$exit_code"
