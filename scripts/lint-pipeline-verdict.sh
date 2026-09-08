#!/usr/bin/env bash
# scripts/lint-pipeline-verdict.sh — ratchet for verdicts derived from a
# pipeline's tail.
#
# In a pipeline the exit status you get is not the exit status you meant:
# `cmd | filter && echo ok` reads the status of the *last* stage (the filter),
# so the verdict prints `ok` no matter what `cmd` did — the diff it just
# printed and the verdict two lines below contradict each other, and a
# confident string is exactly what a scanner searching for "clean" would
# believe. The verdict line must be unreachable when the gate failed: branch
# on the command you care about (`if cmd >/dev/null 2>&1; then ...`) or
# capture its status first, or run the script under `set -euo pipefail`, which
# makes the pipeline itself carry the gate's status.
#
# A site is a non-comment line where a `&&` or `||` command list follows a
# simple pipeline (`|`, or `|&`) on the same line, in a script that never
# enables pipefail. Heredoc payloads and quoted strings are data, never code.
# Line continuations and arithmetic expansions are outside the detector: an
# entry in the allowlist may be such a false positive (regex alternation
# inside `[[ =~ ]]` is one), which is fine for a ratchet — the count can
# only shrink. `|` in case patterns (`a|b) cmd`) is not a site.
#
# This script counts those sites per file and compares them to an allowlist of
# the pre-existing offenders, so the count can only shrink.
#
# Usage:
#   scripts/lint-pipeline-verdict.sh [--root <dir>] [--allowlist <file>]
#   scripts/lint-pipeline-verdict.sh --list       # print every site as <path>:<line>
#   scripts/lint-pipeline-verdict.sh --seed       # rewrite the allowlist from the tree
#   scripts/lint-pipeline-verdict.sh --help
#
# Scans <root>/scripts for *.sh / *.bash at any depth, plus top-level *.sh /
# *.bash at <root>, the scripts that decide what gets merged.
#
# Allowlist format, one line per offending file (blank lines and #-comments ok):
#   <path-relative-to-root> <count>
#
# Exit 0 when every file is at or below its allowlisted count and no unlisted
# file has a site; exit 1 otherwise.

set -euo pipefail

script_dir="$(cd "${0%/*}" 2>/dev/null && pwd -P || pwd -P)"
root="$(cd "${script_dir}/.." && pwd -P)"
allowlist=""
mode="check"

# Print the header comment block (everything from line 2 up to the first
# non-comment line) as the usage text, so --help cannot drift from the header.
usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --root) root="$(cd "$2" && pwd -P)"; shift 2 ;;
        --allowlist) allowlist="$2"; shift 2 ;;
        --list) mode="list"; shift ;;
        --seed) mode="seed"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "lint-pipeline-verdict: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ -n "$allowlist" ] || allowlist="${root}/tests/fixtures/pipeline-verdict-allowlist.txt"

awk_prog="$(mktemp)"
trap 'rm -f "$awk_prog"' EXIT

cat > "$awk_prog" <<'AWK'
# Emit "<file>:<line>" for every line in which a `&&` or `||` command list
# follows a simple pipeline, in a file that never enables pipefail. Heredoc
# payloads are data, never code. Quoted strings are blanked before scanning,
# so `grep "a|b" && c` is not a site.
function hd_delim(line,   i, s, c) {
    i = index(line, "<<")
    if (i == 0) return ""
    if (substr(line, i + 2, 1) == "<") return ""          # here-string, not heredoc
    s = substr(line, i + 2)
    sub(/^-/, "", s)
    sub(/^[ \t]+/, "", s)
    c = substr(s, 1, 1)
    if (c == SQ || c == DQ) {
        s = substr(s, 2)
        i = index(s, c)
        if (i > 0) s = substr(s, 1, i - 1)
    } else if (match(s, /[^A-Za-z0-9_]/)) {
        s = substr(s, 1, RSTART - 1)
    }
    return s
}
# Replace the contents of single- and double-quoted strings (with backslash
# escapes honoured inside double quotes) by nothing.
function strip_quotes(line,   out, i, n, c, q, esc) {
    out = ""; n = length(line); q = ""; esc = 0
    for (i = 1; i <= n; i++) {
        c = substr(line, i, 1)
        if (q == SQ) {
            if (c == SQ) q = ""
        } else if (q == DQ) {
            if (esc) esc = 0
            else if (c == "\\") esc = 1
            else if (c == DQ) q = ""
        } else if (c == SQ) q = SQ
        else if (c == DQ) q = DQ
        else out = out c
    }
    return out
}
# Position of the first simple `|` (the `||` operator does not count; `|&`
# counts as a pipeline).
function first_pipe(line,   n, i, c) {
    n = length(line)
    for (i = 1; i <= n; i++) {
        c = substr(line, i, 1)
        if (c != "|") continue
        if (substr(line, i + 1, 1) == "|") { i += 1; continue }   # `||` operator
        return i
    }
    return 0
}
# A site: a simple `|` with `&&` or `||` anywhere after it on the line.
# A `|` that sits before the first `)` with no `(` before that `)` is a
# case-pattern alternative (`a|b) cmd && ...`), not a pipeline.
function is_site(line,   pipe, rp, op, tail) {
    pipe = first_pipe(line)
    if (pipe == 0) return 0
    rp = index(line, ")")
    op = index(line, "(")
    if (rp > pipe && (op == 0 || op > rp)) return 0
    tail = substr(line, pipe + 1)
    return index(tail, "&&") > 0 || index(tail, "||") > 0
}
BEGIN {
    SQ = sprintf("%c", 39); DQ = sprintf("%c", 34)
    hd = ""; has_pipefail = 0
}
FNR == 1 { hd = ""; has_pipefail = 0 }
hd != "" {
    if ($0 ~ "^[ \t]*" hd "[ \t]*$") hd = ""
    next
}
{
    hd = hd_delim($0)
    if (hd != "") next
    code = strip_quotes($0)
    sub(/#[^#]*$/, "", code)                          # trailing unquoted comment
    if (code ~ /^[[:space:]]*#/) next                 # comment line
    if (code ~ /^[[:space:]]*set[[:space:]]+/ && code ~ /pipefail/) {
        has_pipefail = 1                              # pipeline carries the gate's status
        next
    }
    if (!has_pipefail && is_site(code))
        printf "%s:%d\n", FILENAME, FNR
}
END { }
AWK

collect_files() {
    {
        find "${root}/scripts" -type f \( -name '*.sh' -o -name '*.bash' \) 2>/dev/null
        find "${root}" -maxdepth 1 -type f \( -name '*.sh' -o -name '*.bash' \) 2>/dev/null
    } | LC_ALL=C sort -u
}

# "<relative-path>:<line>" for every pipeline-tail verdict in the tree.
sites() {
    local files
    files="$(collect_files)"
    [ -n "$files" ] || return 0
    printf '%s\n' "$files" | xargs awk -f "$awk_prog" | sed "s#^${root}/##"
}

# "<relative-path> <count>", one line per offending file.
counts() {
    sites | cut -d: -f1 | LC_ALL=C uniq -c | awk '{ print $2, $1 }'
}

case "$mode" in
    list) sites; exit 0 ;;
    seed)
        {
            echo "# Pipeline-tail verdicts that predate the ratchet (issue #3716)."
            echo "# Format: <path> <count>. Regenerate with scripts/lint-pipeline-verdict.sh --seed."
            echo "# A count may only shrink; a new site in any file is a blocking finding."
            counts
        } > "$allowlist"
        echo "lint-pipeline-verdict: seeded $allowlist"
        exit 0
        ;;
esac

failures=0
observed="$(counts)"

while read -r path count; do
    [ -n "$path" ] || continue
    allowed="$(awk -v p="$path" '$1 == p { print $2; exit }' "$allowlist" 2>/dev/null || true)"
    if [ -z "$allowed" ]; then
        echo "PIPELINE_VERDICT:${path}: ${count} pipeline-tail verdict(s) in a file with no allowlist entry"
        failures=$((failures + 1))
    elif [ "$count" -gt "$allowed" ]; then
        echo "PIPELINE_VERDICT:${path}: ${count} pipeline-tail verdict(s), allowlist permits ${allowed}"
        failures=$((failures + 1))
    fi
done <<EOF
$observed
EOF

if [ "$failures" -ne 0 ]; then
    echo "lint-pipeline-verdict: ${failures} file(s) regressed." >&2
    echo "A verdict on the tail of a pipeline is the tail's status, not the gate's." >&2
    echo "Branch on the command you care about, or run the script under 'set -euo pipefail'." >&2
    exit 1
fi

echo "lint-pipeline-verdict: OK ($(printf '%s\n' "$observed" | grep -c . || true) allowlisted file(s), no new sites)"
