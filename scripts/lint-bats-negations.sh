#!/usr/bin/env bash
# scripts/lint-bats-negations.sh — ratchet for mid-body negated Bats assertions.
#
# Bash ignores a failing negated command under `set -e` (POSIX: "the -e setting
# shall be ignored when the command is the `!` reserved word"), and Bats derives
# a test's result from the body's LAST command. A `! cmd` that is not the final
# statement of its @test block is therefore a silent no-op: the assertion cannot
# fail. Rewrite those as `run ! cmd` plus a `$status` check.
#
# This script counts those sites per file and compares them to an allowlist of
# the pre-existing offenders, so the count can only shrink.
#
# Usage:
#   scripts/lint-bats-negations.sh [--root <dir>] [--allowlist <file>]
#   scripts/lint-bats-negations.sh --list       # print every site as <path>:<line>
#   scripts/lint-bats-negations.sh --seed       # rewrite the allowlist from the tree
#   scripts/lint-bats-negations.sh --help
#
# Scans <root>/tests and <root>/skills for *.bats at any depth, so
# skills/<skill>/tests/*.bats is covered as well as tests/**/*.bats.
#
# Allowlist format, one line per offending file (blank lines and #-comments ok):
#   <path-relative-to-root> <count>
#
# Exit 0 when every file is at or below its allowlisted count and no unlisted
# file has a site; exit 1 otherwise.

set -eu

script_dir="$(cd "${0%/*}" 2>/dev/null && pwd -P || pwd -P)"
root="$(cd "${script_dir}/.." && pwd -P)"
allowlist=""
mode="check"

usage() {
    sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --root) root="$(cd "$2" && pwd -P)"; shift 2 ;;
        --allowlist) allowlist="$2"; shift 2 ;;
        --list) mode="list"; shift ;;
        --seed) mode="seed"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "lint-bats-negations: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ -n "$allowlist" ] || allowlist="${root}/tests/fixtures/bats-negation-allowlist.txt"

awk_prog="$(mktemp)"
trap 'rm -f "$awk_prog"' EXIT

cat > "$awk_prog" <<'AWK'
# Emit "<file>:<line>" for every ^\s*! statement inside an @test block that is
# not the block's final statement. Heredoc payloads are data, never statements.
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
function flush(   i, last, l) {
    last = 0
    for (i = 1; i <= n; i++) {
        l = lines[i]
        if (l ~ /^[[:space:]]*$/) continue
        if (l ~ /^[[:space:]]*#/) continue
        last = i
    }
    for (i = 1; i <= n; i++)
        if (lines[i] ~ /^[[:space:]]*!/ && i != last)
            printf "%s:%d\n", FILENAME, lno[i]
    intest = 0
    n = 0
}
BEGIN { SQ = sprintf("%c", 39); DQ = sprintf("%c", 34); intest = 0; n = 0; hd = "" }
FNR == 1 { if (intest) flush(); hd = "" }
hd != "" {
    if ($0 ~ "^[ \t]*" hd "[ \t]*$") hd = ""
    if (intest) { n++; lines[n] = "#heredoc"; lno[n] = FNR }
    next
}
/^[[:space:]]*@test[[:space:]]/ {
    if (intest) flush()
    if ($0 ~ /\{[[:space:]]*$/) { intest = 1; n = 0 }
    hd = hd_delim($0)
    next
}
intest && /^\}[[:space:]]*$/ { flush(); next }
{
    if (intest) { n++; lines[n] = $0; lno[n] = FNR }
    hd = hd_delim($0)
}
END { if (intest) flush() }
AWK

collect_files() {
    find "${root}/tests" "${root}/skills" -type f -name '*.bats' 2>/dev/null | LC_ALL=C sort
}

# "<relative-path>:<line>" for every mid-body negation in the tree.
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
            echo "# Mid-body negated Bats assertions that predate the ratchet (issue #3091)."
            echo "# Format: <path> <count>. Regenerate with scripts/lint-bats-negations.sh --seed."
            echo "# A count may only shrink; a new site in any file is a blocking finding."
            counts
        } > "$allowlist"
        echo "lint-bats-negations: seeded $allowlist"
        exit 0
        ;;
esac

failures=0
observed="$(counts)"

while read -r path count; do
    [ -n "$path" ] || continue
    allowed="$(awk -v p="$path" '$1 == p { print $2; exit }' "$allowlist" 2>/dev/null || true)"
    if [ -z "$allowed" ]; then
        echo "BATS_NEGATION:${path}: ${count} mid-body negated assertion(s) in a file with no allowlist entry"
        failures=$((failures + 1))
    elif [ "$count" -gt "$allowed" ]; then
        echo "BATS_NEGATION:${path}: ${count} mid-body negated assertion(s), allowlist permits ${allowed}"
        failures=$((failures + 1))
    fi
done <<EOF
$observed
EOF

if [ "$failures" -ne 0 ]; then
    echo "lint-bats-negations: ${failures} file(s) regressed." >&2
    echo "A mid-body '! cmd' cannot fail under set -e; rewrite it as 'run ! cmd' + a \$status check." >&2
    exit 1
fi

echo "lint-bats-negations: OK ($(printf '%s\n' "$observed" | grep -c . || true) allowlisted file(s), no new sites)"
