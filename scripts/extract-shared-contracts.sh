#!/usr/bin/env bash
# scripts/extract-shared-contracts.sh — deterministic shared-contract scanner.
#
# Phase 3.75 (prompt-cache-reclaim spec, Phase 2 child D) used to run a full
# Tier-A (opus) subagent to read every child issue body and synthesize a
# `## Shared contracts` summary. Steps 1-2 of that pass — extracting public
# interfaces, signatures, file paths, and naming conventions — are purely
# mechanical. This script does them deterministically (no LLM), so the standalone
# Tier-A pass collapses to: this scan + at most a small Tier-B reconcile for
# genuine conflicts.
#
# It greps each child issue body for three token classes, then reports every
# token that appears in >=2 DISTINCT issues (a real cross-issue contract — a
# token in a single issue is that issue's private business, not a shared one):
#
#   - File paths      : backtick-quoted tokens that look like repo paths
#                       (contain "/" or end in a source extension).
#   - Signatures      : backtick-quoted call tokens — `name(...)`, path-
#                       qualified `Foo::bar(...)`, and generic `name<G>(...)`
#                       (e.g. Rust `parse<T: De>(s)`), so cross-language
#                       interfaces produce tokens.
#   - Names / env vars: ALL-CAPS_SNAKE tokens (>=4 chars), e.g. AUTOSPEC_*.
#
# Cross-language boundary block (epic #3104, child #3111): inside the same
# marker region the script also emits a `## Cross-language boundaries` table
# when the children span >=2 distinct `lang:*` labels (read from each child's
# `## Language fit` block) or any child is `lang:mixed`. Boundary rows are
# declared by the children under their own `## Cross-language boundaries`
# tables (columns: Boundary | Transport | Schema (source of truth) | Owner |
# Golden fixture) and are collected verbatim. Every declared row's Schema cell
# must name an existing file under `schemas/` (resolved against --repo-root);
# a declared row with a missing or out-of-tree schema FAILS CLOSED (exit 3,
# nothing on stdout) rather than emitting an unbacked table.
#
# Usage:
#   extract-shared-contracts.sh <body1.md> <body2.md> ...   # explicit bodies
#   extract-shared-contracts.sh --dir <dir>                 # all *.md in <dir>
#   extract-shared-contracts.sh ... --repo-root <dir>       # root holding schemas/
#   extract-shared-contracts.sh -h | --help
#
# Output (stdout): a `## Shared contracts` markdown block (plus the
# cross-language boundary block when its trigger fires). Deterministic — the
# same inputs always produce byte-identical output (every list is sorted).
#
# Exit codes:
#   0  success (block emitted; may be "none" when there is no cross-issue overlap)
#   2  usage error (no inputs, or an input path is missing/unreadable)
#   3  fail closed (a declared boundary row names a schema that is not an
#      existing file under schemas/)
#
# Conventions: set -u (no -e — we branch explicitly); if/then/fi for one-sided
# conditionals; no RETURN traps (repo bash 3.2 gotchas).
set -u

PROG="$(basename "$0")"

usage() {
    cat <<EOF
$PROG — deterministic shared-contract scanner across child issue bodies.

Usage:
  $PROG <body1.md> <body2.md> ...   Scan the given issue body files.
  $PROG --dir <dir>                 Scan every *.md file in <dir>.
  $PROG ... --repo-root <dir>       Repo root holding schemas/ (default: .).
  $PROG -h | --help                 Show this help.

Emits a '## Shared contracts' markdown block listing every file path,
function signature, and ALL-CAPS name token that appears in >=2 distinct
issues. When the children span >=2 distinct lang:* labels (from each child's
'## Language fit' block) or any child is lang:mixed, a '## Cross-language
boundaries' table is emitted inside the same marker region; declared boundary
rows must name an existing file under schemas/ or the script fails closed
with exit 3 and emits nothing.
Deterministic: identical inputs -> byte-identical output.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# ---- argument parsing -----------------------------------------------------
FILES=()
REPO_ROOT="."
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --dir)
            [ "$#" -ge 2 ] || die 2 "--dir requires a directory argument"
            dir="$2"
            [ -d "$dir" ] || die 2 "not a directory: $dir"
            # Collect *.md deterministically (sorted).
            while IFS= read -r f; do
                FILES+=("$f")
            done < <(find "$dir" -maxdepth 1 -type f -name '*.md' | sort)
            [ "${#FILES[@]}" -gt 0 ] || die 2 "no *.md files in directory: $dir"
            shift 2
            ;;
        --repo-root)
            [ "$#" -ge 2 ] || die 2 "--repo-root requires a directory argument"
            REPO_ROOT="$2"
            [ -d "$REPO_ROOT" ] || die 2 "not a directory: $REPO_ROOT"
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

# Validate every input is a readable file.
for f in "${FILES[@]}"; do
    [ -f "$f" ] && [ -r "$f" ] || die 2 "missing or unreadable input: $f"
done

# ---- token extraction -----------------------------------------------------
# For each token class we build a list of "<token>\t<file-index>" pairs, then
# count DISTINCT file indices per token. A token is "shared" iff it appears in
# >=2 distinct files.

# Emits shared tokens (one per line, sorted) for a given extractor.
# $1 = a function name that, given a file path, prints its tokens (one/line).
shared_tokens() {
    extractor="$1"
    idx=0
    {
        for f in "${FILES[@]}"; do
            # De-dupe within a single file so one issue mentioning a token N
            # times still counts as ONE distinct issue.
            "$extractor" "$f" | sort -u | while IFS= read -r tok; do
                [ -n "$tok" ] && printf '%s\t%s\n' "$tok" "$idx"
            done
            idx=$((idx + 1))
        done
    } | sort -u \
      | awk -F'\t' '{ count[$1]++ } END { for (t in count) if (count[t] >= 2) print t }' \
      | sort
}

# File paths: backtick-quoted tokens that contain "/" or end in a known source
# extension. (Strips the backticks; keeps the path verbatim.)
extract_paths() {
    grep -oE '`[^`]+`' "$1" 2>/dev/null \
        | sed -E 's/^`//; s/`$//' \
        | grep -E '(/|\.(sh|md|mjs|ps1|bats|yml|yaml|json|js|ts|py))$|/' \
        | grep -vE '\(\)$'
}

# Signatures: backtick-quoted call tokens — `name(...)`, path-qualified
# `Foo::bar(...)` (Rust/Go/C++/Java style) and generic `name<G>(...)`
# (e.g. Rust `parse<T: De>(s)`). Cross-language interfaces must produce a
# token, so all three shapes match.
extract_signatures() {
    grep -oE '`[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*(<[^`]*>)?\([^`]*\)`' "$1" 2>/dev/null \
        | sed -E 's/^`//; s/`$//'
}

# Names / env vars: ALL-CAPS_SNAKE tokens of >=4 chars (e.g. AUTOSPEC_FOO,
# RULE_ID). Word-bounded; ignores backtick state (a name is a name either way).
extract_names() {
    grep -oE '\b[A-Z][A-Z0-9]*(_[A-Z0-9]+)+\b' "$1" 2>/dev/null \
        | awk '{ if (length($0) >= 4) print }'
}

# Language: the lang:* label from a body's `## Language fit` block (the
# autospec-language marker region when present, the heading section otherwise).
# Prints one label or nothing.
child_lang() {
    awk '
        /^<!-- autospec-language:begin -->/ { in_s = 1; next }
        /^<!-- autospec-language:end -->/   { in_s = 0; next }
        /^## Language fit$/                 { if (!in_s) in_s = 1; next }
        in_s && /^## /                      { in_s = 0 }
        in_s                                { print }
    ' "$1" \
        | grep -oE 'lang:(rust|go|python|typescript|javascript|java|bash|ruby|csharp|markdown|mixed|unknown)' \
        | head -n 1 \
        | sed 's/^lang://'
}

# Boundary rows: data rows of a body's `## Cross-language boundaries` table
# section. The header row and the |---| separator row are dropped; rows are
# kept verbatim (trimmed).
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

# Schema cell: the 3rd table cell of a "| a | b | c | d | e |" row, trimmed
# and stripped of backticks.
row_schema_cell() {
    printf '%s\n' "$1" \
        | awk -F'|' '{ c = $4; gsub(/^[ \t]+|[ \t]+$/, "", c); print c }' \
        | sed 's/`//g'
}

# ---- cross-language boundary block (epic #3104, child #3111) --------------
# Trigger: the children span >=2 distinct lang:* labels (unknown is excluded —
# an abstention is not a language) or any child is lang:mixed. A declared row
# is a positive assertion of a boundary, so it forces the block even when the
# sibling set is single-language. Declared rows are validated BEFORE anything
# is emitted: a row whose schema cell is not an existing file under schemas/
# fails closed (exit 3, nothing on stdout).
paths="$(shared_tokens extract_paths)"
sigs="$(shared_tokens extract_signatures)"
names="$(shared_tokens extract_names)"

EMIT_BOUNDARY=0
BOUNDARY_ROWS=""
ALL_LANGS=""
ALL_ROWS=""
for f in "${FILES[@]}"; do
    l="$(child_lang "$f")"
    [ -n "$l" ] && ALL_LANGS="${ALL_LANGS}${l}"$'\n'
    r="$(boundary_rows "$f")"
    [ -n "$r" ] && ALL_ROWS="${ALL_ROWS}${r}"$'\n'
done
KNOWN_LANGS="$(printf '%s' "$ALL_LANGS" | grep -v '^unknown$' | grep -v '^$' | sort -u)"
if [ -n "$KNOWN_LANGS" ]; then
    SPAN="$(printf '%s\n' "$KNOWN_LANGS" | wc -l | tr -d ' ')"
    if [ "$SPAN" -ge 2 ] || printf '%s\n' "$KNOWN_LANGS" | grep -qx 'mixed'; then
        EMIT_BOUNDARY=1
    fi
fi
if [ -n "$ALL_ROWS" ]; then
    BOUNDARY_ROWS="$(printf '%s\n' "$ALL_ROWS" | sort -u)"
    MISSING=""
    while IFS= read -r row; do
        [ -n "$row" ] || continue
        cell="$(row_schema_cell "$row")"
        case "$cell" in
            schemas/*)
                if [ ! -f "$REPO_ROOT/$cell" ]; then
                    MISSING="${MISSING}${cell}"$'\n'
                fi
                ;;
            *)
                MISSING="${MISSING}${cell:-<empty schema cell>}"$'\n'
                ;;
        esac
    done <<< "$BOUNDARY_ROWS"
    if [ -n "$MISSING" ]; then
        while IFS= read -r m; do
            [ -n "$m" ] && printf '%s: declared boundary names a schema that is not an existing file under schemas/: %s\n' "$PROG" "$m" >&2
        done <<< "$MISSING"
        exit 3
    fi
    EMIT_BOUNDARY=1
fi

# The marker pair is load-bearing, not decoration: scripts/lint-issue.sh
# exempts generated metadata from the authored word budget only between
# these markers, and the heading must sit inside them to be exempt too.
printf '<!-- autospec-shared-contracts:begin -->\n'
printf '## Shared contracts\n\n'

emit_section() {  # emit_section <heading> <newline-list>
    heading="$1"; list="$2"
    if [ -n "$list" ]; then
        printf '### %s\n\n' "$heading"
        printf '%s\n' "$list" | while IFS= read -r t; do
            [ -n "$t" ] && printf -- '- `%s`\n' "$t"
        done
        printf '\n'
    fi
}

emit_boundary_block() {
    printf '## Cross-language boundaries\n\n'
    printf 'Rules the block encodes: the owning side lands the schema **first**; the consuming issue carries `Depends on issue #N` against it; each boundary gets one golden fixture under `tests/fixtures/` asserted by **both** sides'"'"' own test runners.\n\n'
    printf '| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |\n'
    printf '|---|---|---|---|---|\n'
    if [ -n "$BOUNDARY_ROWS" ]; then
        printf '%s\n' "$BOUNDARY_ROWS" | while IFS= read -r r; do
            [ -n "$r" ] && printf '%s\n' "$r"
        done
        printf '\n'
    fi
}

if [ -z "$paths" ] && [ -z "$sigs" ] && [ -z "$names" ]; then
    printf '_No cross-issue contracts detected (no token appears in >=2 issues)._\n'
    if [ "$EMIT_BOUNDARY" -eq 1 ]; then
        printf '\n'
        emit_boundary_block
    fi
else
    emit_section 'File paths' "$paths"
    emit_section 'Signatures' "$sigs"
    emit_section 'Names / env vars' "$names"
    if [ "$EMIT_BOUNDARY" -eq 1 ]; then
        emit_boundary_block
    fi
fi

printf '<!-- autospec-shared-contracts:end -->\n'

exit 0
