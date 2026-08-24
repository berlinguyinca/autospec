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
#   - Signatures      : backtick-quoted `name(...)` function/identifier calls.
#   - Names / env vars: ALL-CAPS_SNAKE tokens (>=4 chars), e.g. AUTOSPEC_*.
#
# Usage:
#   extract-shared-contracts.sh <body1.md> <body2.md> ...   # explicit bodies
#   extract-shared-contracts.sh --dir <dir>                 # all *.md in <dir>
#   extract-shared-contracts.sh -h | --help
#
# Output (stdout): a `## Shared contracts` markdown block. Deterministic — the
# same inputs always produce byte-identical output (every list is sorted).
#
# Exit codes:
#   0  success (block emitted; may be "none" when there is no cross-issue overlap)
#   2  usage error (no inputs, or an input path is missing/unreadable)
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
  $PROG -h | --help                 Show this help.

Emits a '## Shared contracts' markdown block listing every file path,
function signature, and ALL-CAPS name token that appears in >=2 distinct
issues. Deterministic: identical inputs -> byte-identical output.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# ---- argument parsing -----------------------------------------------------
FILES=()
if [ "$#" -eq 0 ]; then
    usage >&2
    exit 2
fi

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
        ;;
    *)
        for f in "$@"; do
            FILES+=("$f")
        done
        ;;
esac

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

# Signatures: backtick-quoted `identifier(...)` tokens (function/method calls).
extract_signatures() {
    grep -oE '`[A-Za-z_][A-Za-z0-9_]*\([^`]*\)`' "$1" 2>/dev/null \
        | sed -E 's/^`//; s/`$//'
}

# Names / env vars: ALL-CAPS_SNAKE tokens of >=4 chars (e.g. AUTOSPEC_FOO,
# RULE_ID). Word-bounded; ignores backtick state (a name is a name either way).
extract_names() {
    grep -oE '\b[A-Z][A-Z0-9]*(_[A-Z0-9]+)+\b' "$1" 2>/dev/null \
        | awk '{ if (length($0) >= 4) print }'
}

# ---- emit the block -------------------------------------------------------
paths="$(shared_tokens extract_paths)"
sigs="$(shared_tokens extract_signatures)"
names="$(shared_tokens extract_names)"

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

if [ -z "$paths" ] && [ -z "$sigs" ] && [ -z "$names" ]; then
    printf '_No cross-issue contracts detected (no token appears in >=2 issues)._\n'
    printf '<!-- autospec-shared-contracts:end -->\n'
    exit 0
fi

emit_section 'File paths' "$paths"
emit_section 'Signatures' "$sigs"
emit_section 'Names / env vars' "$names"

printf '<!-- autospec-shared-contracts:end -->\n'

exit 0
