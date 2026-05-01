#!/usr/bin/env bash
# scripts/sizing-check.sh — validate a draft issue body against the spec
# §3.4 sizing caps:
#   - Body ≤400 words (counted across the whole body).
#   - Implementation outline ≤30 lines (lines under "## Implementation outline"
#     until the next "## " heading).
#   - Files touched ≤3 (distinct file paths under the outline; counted as
#     bullet lines beginning with "- `<path>`" — backtick-delimited paths).
#
# Usage:
#   scripts/sizing-check.sh < body.md      # read body on stdin
#   cat body.md | scripts/sizing-check.sh
#
# Exit codes:
#   0 — all caps satisfied.
#   non-zero — at least one cap violated. The script prints the failing
#     cap name on a single line to stderr (e.g. `body-words`,
#     `outline-lines`, `outline-files`) and exits 1.

set -eu

WORD_CAP=400
OUTLINE_LINE_CAP=30
OUTLINE_FILE_CAP=3

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

# Slurp stdin into a tmp file so we can scan it multiple times.
TMP="$(mktemp -t sizing-check.XXXXXX)"
trap 'rm -f "$TMP"' EXIT INT TERM
cat > "$TMP"

# 1. Body word count.
words="$(wc -w < "$TMP" | awk '{print $1}')"
if [ "$words" -gt "$WORD_CAP" ]; then
    fail "body-words: ${words} > ${WORD_CAP}"
fi

# 2. Implementation outline line count.
# Extract everything between `## Implementation outline` and the next `## `.
outline_lines="$(awk '
    /^## Implementation outline/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section { print }
' "$TMP" | grep -c -E '\S' || true)"
if [ "$outline_lines" -gt "$OUTLINE_LINE_CAP" ]; then
    fail "outline-lines: ${outline_lines} > ${OUTLINE_LINE_CAP}"
fi

# 3. Distinct file paths in the outline.
# A file path here is any backtick-quoted token on a bullet line that
# starts with "- `". Count distinct values.
outline_files="$(awk '
    /^## Implementation outline/ { in_section = 1; next }
    /^## / && in_section { exit }
    in_section && /^- `/ {
        line = $0
        sub(/^- `/, "", line)
        idx = index(line, "`")
        if (idx > 0) {
            print substr(line, 1, idx - 1)
        }
    }
' "$TMP" | awk 'NF > 0' | sort -u | wc -l | awk '{print $1}')"
if [ "$outline_files" -gt "$OUTLINE_FILE_CAP" ]; then
    fail "outline-files: ${outline_files} > ${OUTLINE_FILE_CAP}"
fi

exit 0
