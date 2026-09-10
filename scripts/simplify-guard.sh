#!/usr/bin/env bash
# scripts/simplify-guard.sh — gate that a simplification diff cannot grow the diff.
#
# A simplification pass is safe only if "must not change behavior" is checked,
# not requested. This checks the shape of the simplification diff against the
# base diff under review:
#   * it must not touch a path absent from the base diff (reaching outside the
#     change under review is a second change)
#   * it must not add a file (any `new file mode` hunk)
#   * it must not be net-additive (additions may not exceed deletions)
#
# Both diffs are read from files, never a pipeline. Prints one
# `SIMPLIFY_GUARD:<path>:<line>: <reason>` per violation; exit 0 silently
# when clean (an empty simplify diff is a no-op). Exit 2 on a missing or
# unreadable diff file or a usage error.
#
# Usage: scripts/simplify-guard.sh --base-diff <file> --simplify-diff <file>
#
# Options: the base diff is the unified diff of the change under review;
# the simplify diff is the unified diff of the proposed simplification pass.
# Exit codes: 0 clean (or empty simplify diff — a no-op), 1 at least one
# violation (one SIMPLIFY_GUARD line per violation on stdout), 2 missing or
# unreadable diff file or usage error.
set -euo pipefail

# Print the header comment block (lines 2–23) as usage text.
usage() { sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; }

die2() { printf 'simplify-guard: %s\n' "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        --base-diff) [ $# -ge 2 ] || die2 "base diff requires a file argument"; BASE_DIFF="$2"; shift 2 ;;
        --simplify-diff) [ $# -ge 2 ] || die2 "simplify diff requires a file argument"; SIMPLIFY_DIFF="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die2 "unknown argument: $1" ;;
    esac
done

[ -n "${BASE_DIFF:-}" ] && [ -n "${SIMPLIFY_DIFF:-}" ] || die2 "need a base diff file and a simplify diff file (see help)"
[ -f "$BASE_DIFF" ] && [ -r "$BASE_DIFF" ] || die2 "cannot read base diff: $BASE_DIFF"
[ -f "$SIMPLIFY_DIFF" ] && [ -r "$SIMPLIFY_DIFF" ] || die2 "cannot read simplify diff: $SIMPLIFY_DIFF"

# An empty simplify diff changes nothing: a no-op.
[ -s "$SIMPLIFY_DIFF" ] || exit 0

# diff_paths FILE — one line per touched path of a unified diff, deduped.
diff_paths() {
    awk '
        $0 ~ /^--- / { p = substr($0, 5) }
        $0 ~ /^\+\+\+ / { p = substr($0, 5) }
        $0 ~ /^--- / || $0 ~ /^\+\+\+ / {
            if (p != "/dev/null") {
                if (length(p) > 1 && substr(p, 1, 1) == "\"" && substr(p, length(p), 1) == "\"")
                    p = substr(p, 2, length(p) - 2)
                sub(/^a\//, "", p); sub(/^b\//, "", p)
                if (!(p in seen)) { seen[p] = 1; print p }
            }
        }
    ' "$1"
}

# file_records FILE — "<line> <path>" for the first header line of each file
#   section of a unified diff (the --- or +++ line that names the path).
file_records() {
    awk '
        $0 ~ /^--- / || $0 ~ /^\+\+\+ / {
            p = substr($0, 5)
            if (p == "/dev/null") next
            if (length(p) > 1 && substr(p, 1, 1) == "\"" && substr(p, length(p), 1) == "\"")
                p = substr(p, 2, length(p) - 2)
            sub(/^a\//, "", p); sub(/^b\//, "", p)
            if (!(p in seen)) { seen[p] = 1; print NR " " p }
        }
    ' "$1"
}

# new_file_records FILE — "<line> <path>" for each `new file mode` line,
#   mapped to the path of the +++ header that follows it.
new_file_records() {
    awk '
        { lines[NR] = $0 }
        END {
            for (i = 1; i <= NR; i++) {
                if (lines[i] !~ /^new file mode /) continue
                p = ""
                for (j = i + 1; j <= NR; j++) {
                    if (lines[j] ~ /^\+\+\+ /) { p = substr(lines[j], 5); break }
                }
                if (length(p) > 1 && substr(p, 1, 1) == "\"" && substr(p, length(p), 1) == "\"")
                    p = substr(p, 2, length(p) - 2)
                sub(/^b\//, "", p); sub(/^a\//, "", p)
                print i " " (p == "" ? "(unknown)" : p)
            }
        }
    ' "$1"
}

base_paths="$(diff_paths "$BASE_DIFF")"
out=""

add_violation() { out="${out}SIMPLIFY_GUARD:$1:$2: $3
"; }

# ── Check 1: no path absent from the base diff ───────────────────────────────
while IFS= read -r rec; do
    [ -z "$rec" ] && continue
    lno="${rec%% *}"
    path="${rec#* }"
    if ! printf '%s\n' "$base_paths" | grep -Fxq -- "$path"; then
        add_violation "$path" "$lno" "path absent from base diff (reaching outside the change under review)"
    fi
done < <(file_records "$SIMPLIFY_DIFF")

# ── Check 2: no new files ────────────────────────────────────────────────────
while IFS= read -r rec; do
    [ -z "$rec" ] && continue
    lno="${rec%% *}"
    path="${rec#* }"
    add_violation "$path" "$lno" "new file (a simplification pass may not add a file)"
done < <(new_file_records "$SIMPLIFY_DIFF")

# ── Check 3: additions must not exceed deletions ─────────────────────────────
counts="$(awk '
    /^\+\+\+ / { next }
    /^--- / { next }
    /^\+/ { a++ }
    /^-/ { d++ }
    END { printf "%d %d", a + 0, d + 0 }
' "$SIMPLIFY_DIFF")"
added="${counts% *}"
deleted="${counts#* }"
if [ "$added" -gt "$deleted" ]; then
    add_violation "$SIMPLIFY_DIFF" 1 "net-additive diff: +${added}/-${deleted} (additions exceed deletions)"
fi

if [ -n "$out" ]; then
    printf '%s' "$out"
    exit 1
fi
exit 0
