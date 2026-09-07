#!/usr/bin/env bash
# scripts/language-axis-audit.sh — Phase 5.5 language-axis audit (epic #3104,
# child #3112).
#
# Deterministic end-of-run audit of the language-selection-axis contract over
# a set of issue body files (no network, no LLM). Per body it checks:
#
#   1. Language fit  — the body carries a `## Language fit` block (the
#                      autospec-language marker region when present, the
#                      heading section otherwise) naming EXACTLY ONE label
#                      from the closed set:
#                      rust|go|python|typescript|javascript|java|bash|ruby|
#                      csharp|markdown|mixed|unknown
#   2. Boundary rows — every row of any `## Cross-language boundaries` table
#                      in the body (columns: Boundary | Transport | Schema
#                      (source of truth) | Owner | Golden fixture) names a
#                      Schema cell that is an existing file under
#                      <repo-root>/schemas/ and a Golden fixture cell that is
#                      an existing file under <repo-root>/. The two-sided
#                      assertion is the only thing that catches drift; a
#                      boundary whose schema or fixture never landed is a gap.
#
# Usage:
#   language-axis-audit.sh --bodies-dir <dir> --repo-root <dir>
#   language-axis-audit.sh <body1.md> ... --repo-root <dir>
#   language-axis-audit.sh -h | --help
#
# Output (stdout): one `GAP <body>: <reason>` line per finding. Findings are
# the output, not an error. Deterministic: identical inputs -> identical
# output.
#
# Exit codes:
#   0  success (zero or more GAP lines emitted)
#   2  usage error (no inputs, or an input path is missing/unreadable)
#
# Conventions: set -u (no -e — we branch explicitly); no associative arrays
# (repo bash 3.2 gotchas).
set -u

PROG="$(basename "$0")"

usage() {
    cat <<EOF
$PROG — Phase 5.5 language-axis audit (epic #3104, child #3112).

Usage:
  $PROG --bodies-dir <dir> --repo-root <dir>   Audit every *.md in <dir>.
  $PROG <body1.md> ... --repo-root <dir>       Audit the given body files.
  $PROG -h | --help                            Show this help.

Checks each body for (1) a `## Language fit` block naming exactly one label
from the closed lang set, and (2) every declared cross-language boundary row
naming an existing file under <repo-root>/schemas/ (schema cell) and an
existing file under <repo-root>/ (golden fixture cell).

Emits one `GAP <body>: <reason>` line per finding. Exit 0 always when the
inputs are valid (findings are the output); exit 2 on usage errors.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

LANG_SET='rust|go|python|typescript|javascript|java|bash|ruby|csharp|markdown|mixed|unknown'
GAPS=""

add_gap() {  # add_gap <body> <reason>
    GAPS="${GAPS}GAP $1: $2"$'\n'
}

# The `## Language fit` section text (marker region when present, heading
# section otherwise).
language_fit_section() {
    awk '
        /^<!-- autospec-language:begin -->/ { in_s = 1; next }
        /^<!-- autospec-language:end -->/   { in_s = 0; next }
        /^## Language fit$/                 { if (!in_s) in_s = 1; next }
        in_s && /^## /                      { in_s = 0 }
        in_s                                { print }
    ' "$1"
}

# Data rows of a `## Cross-language boundaries` table section (header and
# separator rows dropped), kept verbatim.
boundary_rows() {
    awk '
        /^## Cross-language boundaries[ \t]*$/ { in_s = 1; next }
        in_s && /^## / { in_s = 0 }
        in_s && /^\|/ { print }
    ' "$1" \
        | grep -vE '^\|[-|: ]+\|$' \
        | grep -vE '^\|[[:space:]]*Boundary[[:space:]]*\|' \
        | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//'
}

# <n>-th table cell of a "| a | b | c | ... |" row, trimmed, backticks stripped.
table_cell() {  # table_cell <row> <n>
    printf '%s\n' "$1" | awk -F'|' -v n="$2" '{ c = $(n + 1); gsub(/^[ \t]+|[ \t]+$/, "", c); print c }' | sed 's/`//g'
}

check_language_fit() {  # check_language_fit <body>
    f="$1"
    section="$(language_fit_section "$f")"
    if [ -z "$section" ]; then
        add_gap "$f" "no ## Language fit block (classifier never ran or block was stripped)"
        return 0
    fi
    labels="$(printf '%s\n' "$section" | grep -oE "lang:($LANG_SET)" | sed 's/^lang://' | sort -u)"
    n=0
    for x in $labels; do
        n=$((n + 1))
    done
    if [ "$n" -eq 0 ]; then
        add_gap "$f" "## Language fit block names no lang:* label from the closed set"
    elif [ "$n" -gt 1 ]; then
        add_gap "$f" "## Language fit block names $n distinct lang:* labels (exactly one per issue is required)"
    fi
    return 0
}

check_boundary_rows() {  # check_boundary_rows <body>
    f="$1"
    rows="$(boundary_rows "$f")"
    [ -n "$rows" ] || return 0
    while IFS= read -r row; do
        [ -n "$row" ] || continue
        name="$(table_cell "$row" 1)"
        schema="$(table_cell "$row" 3)"
        fixture="$(table_cell "$row" 5)"
        case "$schema" in
            schemas/*)
                if [ ! -f "$REPO_ROOT/$schema" ]; then
                    add_gap "$f" "boundary \"$name\" schema missing on disk: $schema"
                fi
                ;;
            *)
                add_gap "$f" "boundary \"$name\" schema cell not under schemas/: ${schema:-<empty>}"
                ;;
        esac
        if [ -n "$fixture" ]; then
            if [ ! -f "$REPO_ROOT/$fixture" ]; then
                add_gap "$f" "boundary \"$name\" golden fixture missing on disk: $fixture"
            fi
        else
            add_gap "$f" "boundary \"$name\" golden fixture cell is empty (each boundary gets one fixture under tests/fixtures/)"
        fi
    done <<< "$rows"
    return 0
}

# ---- argument parsing -----------------------------------------------------
REPO_ROOT="."
FILES=()
while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --repo-root)
            [ "$#" -ge 2 ] || die 2 "--repo-root requires a directory argument"
            REPO_ROOT="$2"
            [ -d "$REPO_ROOT" ] || die 2 "not a directory: $REPO_ROOT"
            shift 2
            ;;
        --bodies-dir)
            [ "$#" -ge 2 ] || die 2 "--bodies-dir requires a directory argument"
            dir="$2"
            [ -d "$dir" ] || die 2 "not a directory: $dir"
            while IFS= read -r f; do
                FILES+=("$f")
            done < <(find "$dir" -maxdepth 1 -type f -name '*.md' | sort)
            [ "${#FILES[@]}" -gt 0 ] || die 2 "no *.md files in directory: $dir"
            shift 2
            ;;
        -*)
            die 2 "unknown option: $1"
            ;;
        *)
            FILES+=("$1")
            shift
            ;;
    esac
done
[ "${#FILES[@]}" -gt 0 ] || { usage >&2; exit 2; }

for f in "${FILES[@]}"; do
    [ -f "$f" ] && [ -r "$f" ] || die 2 "missing or unreadable input: $f"
done

# ---- audit ----------------------------------------------------------------
for f in "${FILES[@]}"; do
    check_language_fit "$f"
    check_boundary_rows "$f"
done

if [ -n "$GAPS" ]; then
    printf '%s' "$GAPS"
fi

exit 0
