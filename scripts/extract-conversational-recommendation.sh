#!/usr/bin/env bash
# extract-conversational-recommendation.sh — extract the actionable
# recommendation from a prior assistant message, applying:
#   1. section-heading matcher (## Next steps / What to do next / etc.)
#   2. fenced-block matcher (```autospec-next / ```next-prompt)
#   3. numbered-list-with-action-verbs matcher (>=50% imperative)
#   4. you-should / I-suggest / I-recommend / next-step-is sentence matcher
# then concatenates all matches and runs a prompt-injection guard.
#
# Exit codes:
#   0 — extracted block emitted to stdout
#   2 — bad usage
#   3 — empty recommendation (code_health:continue_no_recommendation) OR
#       injection detected (code_health:continue_injection_detected)
#
# The injection guard logs only the category name to stderr; it never
# leaks the offending content.

set -u

usage() {
    cat <<'EOF'
Usage: extract-conversational-recommendation.sh --message <path>

Reads the assistant message at <path> and prints the extracted
recommendation block to stdout. Exits 3 on empty-recommendation or
injection-detected with a code_health:continue_* category logged to
stderr.
EOF
}

MESSAGE_PATH=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --message)
            MESSAGE_PATH="${2:-}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 2
            ;;
        *)
            echo "extract-conversational-recommendation.sh: unknown arg: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ -z "$MESSAGE_PATH" ]; then
    usage >&2
    exit 2
fi

if [ ! -f "$MESSAGE_PATH" ] && [ "$MESSAGE_PATH" != "/dev/stdin" ]; then
    echo "extract-conversational-recommendation.sh: not a file: $MESSAGE_PATH" >&2
    exit 2
fi

MSG="$(cat "$MESSAGE_PATH")"

# -----------------------------------------------------------------------------
# Matcher 1: section headings (case-insensitive via tolower()).
# Headings recognised:
#   ## Next steps
#   ## What to do next
#   ## Remaining work
#   ## Open blockers
#   ## Suggestions
#   ## Proposed follow-ups
# Extract from the heading line through the next "## " heading or end.
# -----------------------------------------------------------------------------
extract_section_headings() {
    awk '
        BEGIN { in_section = 0 }
        {
            low = tolower($0)
        }
        low ~ /^##[[:space:]]+(next steps|what to do next|remaining work|open blockers|suggestions|proposed follow-ups)[[:space:]]*$/ {
            in_section = 1
            print
            next
        }
        /^##[[:space:]]+/ {
            if (in_section) { in_section = 0 }
            next
        }
        {
            if (in_section) print
        }
    ' <<<"$MSG"
}

# -----------------------------------------------------------------------------
# Matcher 2: fenced blocks ```autospec-next / ```next-prompt
# -----------------------------------------------------------------------------
extract_fenced_blocks() {
    awk '
        BEGIN { in_block = 0 }
        /^```(autospec-next|next-prompt)[[:space:]]*$/ {
            in_block = 1
            next
        }
        /^```[[:space:]]*$/ {
            if (in_block) { in_block = 0; next }
            next
        }
        {
            if (in_block) print
        }
    ' <<<"$MSG"
}

# -----------------------------------------------------------------------------
# Matcher 3: numbered lists where >=50% items start with imperative verbs.
# -----------------------------------------------------------------------------
extract_numbered_action_list() {
    awk '
        BEGIN {
            verbs = "^(fix|add|implement|update|review|refactor|ship|merge|remove|delete|create|build|write|test|run|deploy|document|rename|move|split|extract)([[:space:]]|$)"
            block_lines = ""
            total = 0
            imperative = 0
        }
        function flush() {
            if (total > 0 && imperative * 2 >= total) {
                printf "%s", block_lines
            }
            block_lines = ""
            total = 0
            imperative = 0
        }
        /^[[:space:]]*[0-9]+\.[[:space:]]+/ {
            body = $0
            sub(/^[[:space:]]*[0-9]+\.[[:space:]]+/, "", body)
            low = tolower(body)
            total++
            if (match(low, verbs)) imperative++
            block_lines = block_lines $0 "\n"
            next
        }
        /^[[:space:]]*$/ {
            flush()
            next
        }
        {
            flush()
        }
        END { flush() }
    ' <<<"$MSG"
}

# -----------------------------------------------------------------------------
# Matcher 4: "you should" / "I suggest" / "I recommend" / "next step is" /
# "the next thing to do is" sentences plus the following paragraph.
# -----------------------------------------------------------------------------
extract_recommend_sentences() {
    awk '
        BEGIN {
            pat = "(you should|i suggest|i recommend|next step is|the next thing to do is)"
            in_para = 0
        }
        {
            low = tolower($0)
            if (match(low, pat)) {
                in_para = 1
                print
                next
            }
            if (in_para) {
                if ($0 ~ /^[[:space:]]*$/) {
                    in_para = 0
                } else {
                    print
                }
            }
        }
    ' <<<"$MSG"
}

S1="$(extract_section_headings)"
S2="$(extract_fenced_blocks)"
S3="$(extract_numbered_action_list)"
S4="$(extract_recommend_sentences)"

COMBINED=""
append_if_nonempty() {
    local part="$1"
    part="$(printf '%s' "$part" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    if [ -n "$part" ]; then
        if [ -n "$COMBINED" ]; then
            COMBINED="$COMBINED"$'\n\n'"$part"
        else
            COMBINED="$part"
        fi
    fi
}

append_if_nonempty "$S1"
append_if_nonempty "$S2"
append_if_nonempty "$S3"
append_if_nonempty "$S4"

if [ -z "$COMBINED" ]; then
    echo "code_health:continue_no_recommendation" >&2
    echo "No actionable recommendation found in the last assistant message. Provide an explicit prompt: /continue \"<your prompt>\"." >&2
    exit 3
fi

# -----------------------------------------------------------------------------
# Prompt-injection guard. Categories (logged WITHOUT offending content):
#   prose_override   — "ignore previous" / "disregard prior" / "you are now" / "system prompt"
#   shell_metachar   — line begins with $( , ` , && , ; , | , >
# -----------------------------------------------------------------------------
check_injection() {
    local text="$1"
    if printf '%s' "$text" | grep -qiE '(ignore previous|disregard prior|you are now|system prompt)'; then
        echo "code_health:continue_injection_detected category=prose_override" >&2
        return 1
    fi
    # Single backtick at line start is treated as a shell command-substitution
    # token. Triple-backtick fences (markdown code fences) are NOT flagged.
    # Pipe '|' is matched only when NOT part of a markdown table row, which
    # we approximate by requiring no leading whitespace before alternatives
    # that would otherwise be ambiguous; we keep the strict spec list here.
    if printf '%s\n' "$text" | grep -qE '^[[:space:]]*(\$\(|&&|;|\||>)'; then
        echo "code_health:continue_injection_detected category=shell_metachar" >&2
        return 1
    fi
    if printf '%s\n' "$text" | grep -qE '^[[:space:]]*`([^`]|$)'; then
        echo "code_health:continue_injection_detected category=shell_metachar" >&2
        return 1
    fi
    return 0
}

if ! check_injection "$COMBINED"; then
    echo "Extracted block rejected by the prompt-injection guard." >&2
    exit 3
fi

printf '%s\n' "$COMBINED"
