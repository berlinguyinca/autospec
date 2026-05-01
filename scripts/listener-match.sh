#!/usr/bin/env bash
# scripts/listener-match.sh — classify a chat phrase as an autospec-listen
# trigger.
#
# Usage:
#   scripts/listener-match.sh "<phrase>"     # phrase as $1
#   echo "<phrase>" | scripts/listener-match.sh   # phrase on stdin
#
# Prints exactly one of: issue, spec, none. Always exits 0.
#
# The set of trigger phrases is sourced from
# `skills/autospec-listen/references/trigger-keywords.md` so the script and
# the documentation can never drift. Matching is case-insensitive and
# word-boundary anchored. Bare nouns ("issue", "spec", "ticket") are NOT
# triggers.

set -eu

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIGGERS_MD="$REPO_ROOT/skills/autospec-listen/references/trigger-keywords.md"

if [ ! -f "$TRIGGERS_MD" ]; then
    printf 'listener-match: missing %s\n' "$TRIGGERS_MD" >&2
    printf 'none\n'
    exit 0
fi

# Read trigger phrases out of trigger-keywords.md. Sections are delimited by
# `## Issue triggers` and `## Spec triggers` H2 headings; each phrase appears
# on its own bullet line wrapped in backticks.
parse_trigger_md() {
    section_label="$1"
    awk -v label="$section_label" '
        /^## / {
            in_section = 0
            if (index($0, "## " label) == 1) {
                in_section = 1
            }
            next
        }
        in_section && /^- `[^`]+`$/ {
            line = $0
            sub(/^- `/, "", line)
            sub(/`$/, "", line)
            print line
        }
    ' "$TRIGGERS_MD"
}

# Lower-case a string portably.
to_lower() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

# Test whether $1 (lower-cased) contains any trigger phrase from the list on
# stdin, with word-boundary anchoring (a non-word char OR start/end of input
# on either side of the match).
match_against_list() {
    needle="$1"
    while IFS= read -r phrase; do
        [ -n "$phrase" ] || continue
        phrase_lc="$(to_lower "$phrase")"
        # Word-boundary anchored: either the phrase appears at start/end OR
        # is surrounded by characters that are NOT [a-z0-9_-].
        # Use awk to check; portable across bash/zsh on macOS and Linux.
        if printf '%s' "$needle" | awk -v p="$phrase_lc" '
            BEGIN { found = 0 }
            {
                s = $0
                pl = length(p)
                sl = length(s)
                for (i = 1; i <= sl - pl + 1; i++) {
                    if (substr(s, i, pl) == p) {
                        # Left boundary: at start, or non-word char.
                        if (i == 1) {
                            left_ok = 1
                        } else {
                            lc = substr(s, i - 1, 1)
                            left_ok = (lc !~ /[A-Za-z0-9_-]/) ? 1 : 0
                        }
                        # Right boundary: at end, or non-word char.
                        end_pos = i + pl
                        if (end_pos > sl) {
                            right_ok = 1
                        } else {
                            rc = substr(s, end_pos, 1)
                            right_ok = (rc !~ /[A-Za-z0-9_-]/) ? 1 : 0
                        }
                        if (left_ok && right_ok) {
                            found = 1
                            exit
                        }
                    }
                }
            }
            END { exit found ? 0 : 1 }
        '; then
            return 0
        fi
    done
    return 1
}

match_phrase() {
    candidate_lc="$(to_lower "$1")"

    if match_against_list "$candidate_lc" <<EOF
$(parse_trigger_md "Issue triggers")
EOF
    then
        printf 'issue\n'
        return 0
    fi

    if match_against_list "$candidate_lc" <<EOF
$(parse_trigger_md "Spec triggers")
EOF
    then
        printf 'spec\n'
        return 0
    fi

    printf 'none\n'
}

main() {
    if [ "$#" -ge 1 ]; then
        match_phrase "$1"
    else
        # Read all of stdin into one buffer so multi-line input still works.
        candidate="$(cat)"
        match_phrase "$candidate"
    fi
}

main "$@"
