#!/usr/bin/env bash
# scripts/lint-scratch-promotion.sh — report a tool invoked from a scratch path
# more than twice as a promotion candidate (issue #3977).
#
# The supervisor's working directory (a job-scoped temp dir, /tmp, ...) is not
# a repository: it has no history, no test, no owner, and it expires. A tool
# written there and used once is legitimately throwaway. A tool used more than
# twice is a tool, and belongs in the repository — so this gate reports it for
# promotion (or an explicit discard). The gate is the machine-checkable form of
# the "nothing valuable expires by default" invariant from #3977.
#
# An *invocation site* is a line that runs a scratch-path script: a
# whitespace-delimited token that
#   1. starts with a scratch prefix — /tmp/ /var/tmp/ /private/tmp/
#      ${TMPDIR}/ $TMPDIR/ — and
#   2. ends with a script extension — .sh .bash .py .bats .rb .pl .js .mjs .ts —
#      and
#   3. is in a command position: it is the first token of a command (start of
#      line, or right after ; & && || | ( )), or it immediately follows a
#      launcher (bash sh zsh dash ksh ash python[0-9]* perl ruby node nodejs
#      source . exec env sudo nohup xargs time nice stdbuf command).
#
# A scratch token that is a *data* path — an output redirect target, or the
# argument to a file utility such as rm/mv/cp/cat/touch — is not an invocation
# and is not counted. A scratch token that is an ephemeral mktemp-style
# template (contains XXXXXX or $$) is a per-run temp file, not a promotion
# candidate, and is exempt.
#
# The count is corpus-wide per distinct tool path (a session's command log or
# the repository harness). A tool path whose invocation-site count is greater
# than twice (> 2, i.e. >= 3) is a finding.
#
# Usage:
#   scripts/lint-scratch-promotion.sh [PATH...]
#   scripts/lint-scratch-promotion.sh --root DIR [PATH...]
#   scripts/lint-scratch-promotion.sh --list
#
# With no PATH arguments the scan covers *.sh and *.bats under
# <ROOT>/{scripts,skills} — the harness code a supervision session actually
# runs tools from. (The test fixtures under tests/ are test data that
# deliberately contain scratch-path invocations to exercise this gate, so they
# are not part of the default sweep; pass a real session log as an explicit
# PATH argument instead.) --root overrides ROOT (default: the repository root,
# one level above this script). PATH may be a file or a directory (scanned
# recursively for *.sh, *.bats).
#
# Output (blocking mode): one finding per scratch tool invoked more than
# twice, citing its first invocation site:
#   SCRATCH_PROMOTION:<path>:<line>: <tool> invoked <N>x from a scratch path
#   (first seen here): promote it into the repo or discard it
# --list emits an audit line for every scratch tool found (no threshold):
#   SCRATCH_TOOL:<tool>: <N>x
#
# Exit code (blocking mode) = number of findings (0 = pass), capped at 64.
# --list always exits 0.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

THRESHOLD=2   # report a tool invoked MORE THAN this many times

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die() {
    printf 'ERROR: %s\n' "$2" >&2
    exit "$1"
}

# ---- argument parsing -------------------------------------------------------

LIST_MODE=0
paths=()
while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --list)
            LIST_MODE=1
            shift
            ;;
        --root)
            [ $# -ge 2 ] || die 2 "--root requires a directory argument"
            ROOT_DIR=$(cd "$2" && pwd)
            shift 2
            ;;
        --root=*)
            ROOT_DIR=$(cd "${1#--root=}" && pwd)
            shift
            ;;
        -*)
            die 2 "unknown option: $1"
            ;;
        *)
            paths+=("$1")
            shift
            ;;
    esac
done

if [ ${#paths[@]} -eq 0 ]; then
    for d in scripts skills; do
        [ -d "$ROOT_DIR/$d" ] && paths+=("$ROOT_DIR/$d")
    done
fi
[ ${#paths[@]} -gt 0 ] || die 2 "no paths to scan (pass file or directory paths)"

# ---- file collection --------------------------------------------------------

files=()
for p in "${paths[@]}"; do
    [ -e "$p" ] || die 2 "no such file or directory: $p"
    if [ -f "$p" ]; then
        case "$p" in
            *.sh|*.bats) files+=("$p") ;;
            *) continue ;;
        esac
    elif [ -d "$p" ]; then
        while IFS= read -r f; do
            files+=("$f")
        done < <(find "$p" -type f \( -name '*.sh' -o -name '*.bats' \) | LC_ALL=C sort)
    else
        die 2 "not a file or directory: $p"
    fi
done

if [ ${#files[@]} -eq 0 ]; then
    printf 'INFO: lint-scratch-promotion: no .sh/.bats files to scan\n'
    exit 0
fi

# ---- invocation-site extraction --------------------------------------------
# Emits one line per invocation site:  <relfile>:<line>\t<toolpath>

AWK_PROG=$(cat <<'AWK'
function stripq(s) {
    if (s ~ /^".*"$/ || s ~ /^'.*'$/) s = substr(s, 2, length(s) - 2)
    return s
}
function is_launcher(s) {
    return (s == "bash" || s == "sh" || s == "zsh" || s == "dash" || s == "ksh" \
        || s == "ash" || s == "python" || s == "python2" || s == "python3" \
        || s == "perl" || s == "ruby" || s == "node" || s == "nodejs" \
        || s == "source" || s == "." || s == "exec" || s == "env" \
        || s == "sudo" || s == "nohup" || s == "xargs" || s == "time" \
        || s == "nice" || s == "stdbuf" || s == "command")
}
{
    line = $0
    t = line; sub(/^[ \t]+/, "", t)
    if (t ~ /^#/) next                # pure comment line
    n = split(line, toks, /[ \t]+/)
    at_start = 1
    prev = ""
    for (i = 1; i <= n; i++) {
        tok = toks[i]
        if (tok == "") continue
        if (tok == ";" || tok == "&" || tok == "&&" || tok == "||" \
            || tok == "|" || tok == "(" || tok == ")") {
            prev = ""; at_start = 1; continue
        }
        st = stripq(tok)
        is_scratch = (st ~ /^(\/tmp\/|\/var\/tmp\/|\/private\/tmp\/|\$\{TMPDIR\}\/|\$TMPDIR\/)/) \
            && (st ~ /\.(sh|bash|py|bats|rb|pl|js|mjs|ts)$/)
        ephemeral = (st ~ /XXXXXX/ || st ~ /\$\$/)
        if (is_scratch && !ephemeral) {
            if (at_start || is_launcher(prev))
                printf "%s:%d\t%s\n", FILE, NR, st
        }
        prev = st
        at_start = 0
    }
}
AWK
)

# Collect every invocation site across the corpus.
sites=""
for file in "${files[@]}"; do
    rel="$file"
    case "$rel" in
        "$ROOT_DIR"/*) rel="${rel#"$ROOT_DIR"/}" ;;
    esac
    out=$(LC_ALL=C awk -v FILE="$rel" "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    [ -n "$out" ] || continue
    sites="${sites}${out}
"
done

if [ -z "$sites" ]; then
    exit 0
fi

# ---- report ----------------------------------------------------------------

if [ "$LIST_MODE" -eq 1 ]; then
    printf '%s' "$sites" | LC_ALL=C awk '
        BEGIN { FS = "\t" }
        { c[$2]++ }
        END {
            for (tool in c)
                printf "SCRATCH_TOOL:%s: %dx\n", tool, c[tool]
        }' | LC_ALL=C sort -t: -k2,2 -k1,1
    exit 0
fi

findings=$(printf '%s' "$sites" | LC_ALL=C awk -v THRESH="$THRESHOLD" '
    BEGIN { FS = "\t" }
    {
        tool = $2; site = $1
        c[tool]++
        if (!(tool in first)) first[tool] = site
    }
    END {
        for (tool in c)
            if (c[tool] > THRESH)
                printf "SCRATCH_PROMOTION:%s: %s invoked %dx from a scratch path (first seen here): promote it into the repo or discard it\n", first[tool], tool, c[tool]
    }' | LC_ALL=C sort)

if [ -n "$findings" ]; then
    printf '%s\n' "$findings"
    count=$(printf '%s\n' "$findings" | grep -c '^SCRATCH_PROMOTION:')
    [ "$count" -gt 64 ] && count=64
    exit "$count"
fi
exit 0
