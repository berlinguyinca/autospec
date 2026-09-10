#!/usr/bin/env bash
# build-changelog.sh — fold changelog.d/ fragments into CHANGELOG.md (the release step).
#
# Each agent writes one fragment named after its issue (see changelog.d/README.md);
# fragments are distinct files, so concurrent agents never conflict on CHANGELOG.md.
# This script concatenates the fragments into the top of the ## [Unreleased] section
# and then clears the directory. It is the single place CHANGELOG.md is edited.
#
# Usage:
#   build-changelog.sh             # fold fragments into CHANGELOG.md, then remove them
#   build-changelog.sh --dry-run   # print the resulting [Unreleased] section; change nothing
#   build-changelog.sh -h|--help
#
# Exit codes:
#   0 success (fragments folded, or no fragments present)
#   1 usage / infrastructure error
#
# bash 3.2 safe (this repo's macOS floor): no associative arrays, no mapfile, no ${var,,}.

set -u

usage() { sed -n '2,15p' "$0"; }

DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *)
            printf 'error: unknown argument: %s\n\n' "$arg" >&2
            usage >&2
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"
FRAG_DIR="$REPO_ROOT/changelog.d"

if [ ! -f "$CHANGELOG" ]; then
    printf 'error: no CHANGELOG.md at %s\n' "$CHANGELOG" >&2
    exit 1
fi

if ! grep -q '^## \[Unreleased\]' "$CHANGELOG"; then
    printf 'error: %s has no "## [Unreleased]" section\n' "$CHANGELOG" >&2
    exit 1
fi

# Collect fragment files (markdown, sorted by name = issue order), skipping README.md.
fragments=""
if [ -d "$FRAG_DIR" ]; then
    fragments="$(cd "$FRAG_DIR" && find . -maxdepth 1 -type f -name '*.md' ! -name 'README.md' \
        | sed 's|^\./||' | sort)"
fi

if [ -z "$fragments" ]; then
    printf 'no fragments in changelog.d/ — CHANGELOG.md unchanged\n'
    exit 0
fi

# Concatenate fragments (sorted) into one block, separated by exactly one blank line.
# Each fragment is trimmed of trailing newlines so the join is deterministic regardless
# of whether a fragment file ends with a newline.
block_file="$(mktemp "${TMPDIR:-/tmp}/changelog-block.XXXXXX")"
: > "$block_file"
count=0
first=1
while IFS= read -r name; do
    [ -z "$name" ] && continue
    path="$FRAG_DIR/$name"
    [ -f "$path" ] || continue
    body="$(cat "$path")"
    if [ "$first" = 1 ]; then first=0; else printf '\n\n' >> "$block_file"; fi
    printf '%s' "$body" >> "$block_file"
    count=$((count + 1))
done <<EOF
$fragments
EOF
printf '\n' >> "$block_file"

# Splice the block immediately after the "## [Unreleased]" heading.
out_file="$(mktemp "${TMPDIR:-/tmp}/changelog-out.XXXXXX")"
awk -v blockfile="$block_file" '
    /^## \[Unreleased\]/ {
        print
        printf "\n"
        while ((getline line < blockfile) > 0) { print line }
        close(blockfile)
        next
    }
    { print }
' "$CHANGELOG" > "$out_file"

if [ "$DRY_RUN" = 1 ]; then
    printf 'would fold %s fragment(s) into the top of ## [Unreleased] (preview):\n\n' "$count"
    sed -n '/^## \[Unreleased\]/,/^## \[/p' "$out_file"
    rm -f -- "$block_file" "$out_file"
    exit 0
fi

# Replace CHANGELOG.md in place (truncate + write preserves inode and permissions).
cat "$out_file" > "$CHANGELOG"
rm -f -- "$out_file"

# Clear the fragments (leave README.md and the directory itself).
while IFS= read -r name; do
    [ -z "$name" ] && continue
    rm -f -- "$FRAG_DIR/$name"
done <<EOF
$fragments
EOF
rm -f -- "$block_file"

printf 'folded %s fragment(s) into CHANGELOG.md and cleared changelog.d/:\n' "$count"
printf '%s\n' "$fragments" | sed 's/^/  - /'
