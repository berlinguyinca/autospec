#!/usr/bin/env bash
# scripts/lint-signal-semantics.sh — flag shell pipelines that make an error
# indistinguishable from a zero result (issue #4088).
#
# `cmd 2>/dev/null | wc -l` (and the `| grep -c` variants) collapses two
# different facts into one number: the command failed, and the command ran
# fine and found nothing. A downstream count check then reads "0" as
# "healthy" when it actually means "unmeasured". A signal whose failure mode
# is invisible is a signal that was never consumed.
#
# A line is a finding when, on the same line, a stderr discard to
# /dev/null (`2>/dev/null`, `2>>/dev/null`, optionally spaced) is followed by
# a counting reducer (`| wc ...` or `| grep -c...`). Non-counting reducers
# (`head`, `tail`, plain `grep`, `awk`) are out of scope: they do not turn a
# failure into a number that passes a threshold check.
#
# Waiver: the same line or the line immediately above carries
# `# linter:allow-SIGNAL_SEMANTICS <reason>`. The reason is mandatory; a bare
# marker is rejected and the line stays flagged. Waived lines emit an
# advisory INFO:SIGNAL_SEMANTICS:... line for audit.
#
# Usage:
#   scripts/lint-signal-semantics.sh [PATH...]
#   scripts/lint-signal-semantics.sh --help
#
# With no arguments the scan covers scripts/ and .github/workflows/ at the
# repository root. PATH may be a file or a directory (scanned recursively for
# *.sh, *.bats, *.yml, *.yaml).
#
# Output: one finding per line on stdout:
#   SIGNAL_SEMANTICS:<path>:<line>: <line text>
#
# Exit code = number of blocking findings (0 = pass), capped at 64.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

ALLOW_MARKER='linter:allow-SIGNAL_SEMANTICS'

AWK_PROG=$(cat <<'AWK'
{ lines[NR] = $0 }
END {
    prev_allow = ""
    for (i = 1; i <= NR; i++) {
        t = lines[i]
        sub(/^[ \t]+/, "", t)
        sub(/[ \t]+$/, "", t)
        allow = ""
        if (index(t, MARKER) > 0) {
            rest = t
            sub(".*" MARKER "[ \t]*", "", rest)
            if (rest != "") allow = rest
        }
        if (t != "" && t !~ /^#/ && match(t, /2>>?[ \t]*\/dev\/null/) > 0) {
            after = substr(t, RSTART + RLENGTH)
            if (after ~ /\|[ \t]*wc([^a-z0-9]|$)/ || after ~ /\|[ \t]*grep[ \t]+-c[A-Za-z]*/) {
                if (allow != "") {
                    printf "INFO:SIGNAL_SEMANTICS:%s:%d: waived: %s\n", FILE, i, allow
                    prev_allow = ""
                    continue
                }
                if (prev_allow != "") {
                    printf "INFO:SIGNAL_SEMANTICS:%s:%d: waived: %s\n", FILE, i, prev_allow
                    prev_allow = ""
                    continue
                }
                printf "FIND:%d:%s\n", i, lines[i]
            }
        }
        prev_allow = allow
    }
}
AWK
)

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die() {
    printf 'ERROR: %s\n' "$2" >&2
    exit "$1"
}

# ---- argument parsing -------------------------------------------------------

paths=()
for arg in "$@"; do
    case "$arg" in
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            die 2 "unknown option: $arg"
            ;;
        *)
            paths+=("$arg")
            ;;
    esac
done

if [ ${#paths[@]} -eq 0 ]; then
    [ -d "$ROOT_DIR/scripts" ] && paths+=("$ROOT_DIR/scripts")
    [ -d "$ROOT_DIR/.github/workflows" ] && paths+=("$ROOT_DIR/.github/workflows")
fi
[ ${#paths[@]} -gt 0 ] || die 2 "no paths to scan (pass file or directory paths)"

# ---- file collection --------------------------------------------------------

files=()
for p in "${paths[@]}"; do
    [ -e "$p" ] || die 2 "no such file or directory: $p"
    if [ -f "$p" ]; then
        case "$p" in
            *.sh|*.bats|*.yml|*.yaml) files+=("$p") ;;
            *) continue ;;
        esac
    elif [ -d "$p" ]; then
        while IFS= read -r f; do
            files+=("$f")
        done < <(find "$p" -type f \( -name '*.sh' -o -name '*.bats' -o -name '*.yml' -o -name '*.yaml' \) | LC_ALL=C sort)
    else
        die 2 "not a file or directory: $p"
    fi
done

findings=0

if [ ${#files[@]} -eq 0 ]; then
    printf 'INFO: lint-signal-semantics: no .sh/.bats/.yml/.yaml files to scan\n'
    exit 0
fi

for file in "${files[@]}"; do
    rel="$file"
    case "$rel" in
        "$ROOT_DIR"/*) rel="${rel#"$ROOT_DIR"/}" ;;
    esac

    out=$(LC_ALL=C awk \
        -v FILE="$rel" \
        -v MARKER="$ALLOW_MARKER" \
        "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    [ -n "$out" ] || continue

    while IFS= read -r cand; do
        case "$cand" in
            FIND:*)
                rest="${cand#FIND:}"
                ln="${rest%%:*}"
                text="${rest#*:}"
                printf 'SIGNAL_SEMANTICS:%s:%s: %s\n' "$rel" "$ln" "$text"
                findings=$((findings + 1))
                ;;
            *)
                printf '%s\n' "$cand"
                ;;
        esac
    done <<< "$out"
done

[ "$findings" -gt 64 ] && findings=64
exit "$findings"
