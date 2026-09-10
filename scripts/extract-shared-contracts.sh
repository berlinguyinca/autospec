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
#   extract-shared-contracts.sh --languages <csv> ...       # see below
#   extract-shared-contracts.sh -h | --help
#
# --languages <csv> (issue #3210): comma-separated sibling language labels, e.g.
#   --languages rust,typescript   or   --languages lang:rust,lang:mixed
# When the labels span 2+ distinct languages (or any child carries `mixed`),
# the emitted block additionally contains a `## Cross-language boundaries`
# table. Rows are deterministic: one per `schemas/*.schema.json` path that
# appears in >=2 distinct child bodies (the schema cell is the source of
# truth; the other cells are `-` placeholders the orchestrator fills in). If
# any row's schema file does not exist relative to the CWD, the script fails
# closed — it prints one `PHASE_3_75_FAILED rule=boundary-schema-missing
# path=<schema>` line per missing file to stderr, writes NOTHING to stdout,
# and exits 1.
#
# Output (stdout): a `## Shared contracts` markdown block (plus the
# boundaries block when triggered). Deterministic — the same inputs always
# produce byte-identical output (every list is sorted).
#
# Exit codes:
#   0  success (block emitted; may be "none" when there is no cross-issue overlap)
#   1  fail-closed: a boundary row's schema file does not exist on disk
#   2  usage error (no inputs, an unknown option, or an input path is missing/unreadable)
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
  $PROG --languages <csv> ...       Sibling language labels (comma-separated;
                                    'lang:' prefixes are normalized away). When
                                    the labels span 2+ distinct languages or any
                                    child carries 'mixed', also emit a
                                    '## Cross-language boundaries' table.
  $PROG -h | --help                 Show this help.

Emits a '## Shared contracts' markdown block listing every file path,
function signature, and ALL-CAPS name token that appears in >=2 distinct
issues. Deterministic: identical inputs -> byte-identical output.

Exit codes: 0 success, 1 fail-closed boundary-schema-missing, 2 usage error.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# ---- argument parsing -----------------------------------------------------
# Options may appear in any position alongside the body files. `--` ends
# option parsing (everything after it is a body file).
FILES=()
LANG_CSV=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            for f in "$@"; do
                FILES+=("$f")
            done
            break
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
        --languages)
            [ "$#" -ge 2 ] || die 2 "--languages requires a CSV argument"
            LANG_CSV="$2"
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

# Bodies are optional when --languages is given (the boundaries frame can be
# emitted standalone); but at least one of the two must be present.
if [ "${#FILES[@]}" -eq 0 ] && [ -z "$LANG_CSV" ]; then
    usage >&2
    exit 2
fi

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

# Boundary schemas (#3210): `schemas/*.schema.json` paths, backticks optional.
# A path shared by >=2 distinct child bodies is a cross-language boundary.
extract_schemas() {
    grep -oE 'schemas/[A-Za-z0-9._/-]+\.schema\.json' "$1" 2>/dev/null
}

# ---- language labels -> boundary trigger ---------------------------------
# Normalize the CSV: split on commas, trim whitespace, strip the optional
# `lang:` prefix, drop empties, de-duplicate. The boundary block is triggered
# by 2+ distinct languages OR any child carrying `mixed` (a mixed child IS a
# boundary, even if it is the only one).
DISTINCT_LANGS=""
if [ -n "$LANG_CSV" ]; then
    DISTINCT_LANGS="$(printf '%s' "$LANG_CSV" | tr ',' '\n' \
        | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/^lang://' \
        | grep -v '^[[:space:]]*$' | sort -u)"
fi
BOUNDARIES=0
if [ -n "$DISTINCT_LANGS" ]; then
    n_langs="$(printf '%s\n' "$DISTINCT_LANGS" | wc -l | tr -d '[:space:]')"
    if [ "$n_langs" -ge 2 ] || printf '%s\n' "$DISTINCT_LANGS" | grep -qx 'mixed'; then
        BOUNDARIES=1
    fi
fi

# ---- emit the block -------------------------------------------------------
# bash 3.2 + set -u: expanding an empty array is an error, so only call the
# token scanners when there is at least one body file.
if [ "${#FILES[@]}" -gt 0 ]; then
    paths="$(shared_tokens extract_paths)"
    sigs="$(shared_tokens extract_signatures)"
    names="$(shared_tokens extract_names)"
    shared_schemas="$(shared_tokens extract_schemas)"
else
    paths=""
    sigs=""
    names=""
    shared_schemas=""
fi

# Fail-closed schema check (#3210): every emitted boundary row must point at
# a schema that exists relative to the CWD. This runs BEFORE any stdout is
# written — a failed run must leave the orchestrator with nothing to splice.
if [ "$BOUNDARIES" -eq 1 ] && [ -n "$shared_schemas" ]; then
    schema_missing=0
    while IFS= read -r schema; do
        [ -n "$schema" ] || continue
        if [ ! -f "$schema" ]; then
            printf '%s: PHASE_3_75_FAILED rule=boundary-schema-missing path=%s\n' "$PROG" "$schema" >&2
            schema_missing=1
        fi
    done <<EOF
$shared_schemas
EOF
    [ "$schema_missing" -eq 0 ] || exit 1
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

if [ -z "$paths" ] && [ -z "$sigs" ] && [ -z "$names" ] && [ "$BOUNDARIES" -ne 1 ]; then
    printf '_No cross-issue contracts detected (no token appears in >=2 issues)._\n'
    printf '<!-- autospec-shared-contracts:end -->\n'
    exit 0
fi

emit_section 'File paths' "$paths"
emit_section 'Signatures' "$sigs"
emit_section 'Names / env vars' "$names"

# The cross-language boundary table lives inside the marker region (so the
# word-budget exemption covers it) and after the generic sections. Rows come
# only from shared `schemas/*.schema.json` paths; the non-schema cells are `-`
# placeholders the orchestrator fills from the child bodies (never empty).
emit_boundaries() {  # emit_boundaries <newline-list of shared schema paths>
    list="$1"
    printf '## Cross-language boundaries\n\n'
    printf '| Boundary | Transport | Schema (source of truth) | Owner | Golden fixture |\n'
    printf '|---|---|---|---|---|\n'
    if [ -n "$list" ]; then
        printf '%s\n' "$list" | while IFS= read -r schema; do
            [ -n "$schema" ] || continue
            stem="${schema##*/}"
            stem="${stem%.schema.json}"
            printf '| %s | - | %s | - | - |\n' "$stem" "$schema"
        done
    else
        printf '\n_No shared boundary schemas detected — no `schemas/*.schema.json` path appears in >=2 child bodies; fill in one row per interface before merging the spec._\n'
    fi
    printf '\n'
}

if [ "$BOUNDARIES" -eq 1 ]; then
    emit_boundaries "$shared_schemas"
fi

printf '<!-- autospec-shared-contracts:end -->\n'

exit 0
