#!/usr/bin/env bash
# extract-conversational-recommendation.sh — extract the actionable
# recommendation from a prior assistant message, applying:
#   1. section-heading matcher (## Next steps / What to do next / etc.)
#   2. fenced-block matcher (```autospec-next / ```next-prompt)
#   3. numbered-list-with-action-verbs matcher (>=50% imperative)
#   3.5 Next-prefix / continuation prefixes (Next best slice:, Next step:,
#       Continue with:, Proceed with:, Move on to:, etc. — issue #707)
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

# Shared matcher library (issue #707) — sourced for both this script and
# scripts/refine-prompt.sh::run_continue_loop(). Defines:
#   extract_section_headings, extract_fenced_blocks,
#   extract_numbered_action_list, extract_next_prefix_continuations,
#   extract_recommend_sentences
_MATCHER_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/extract-matchers.sh"
if [ ! -f "$_MATCHER_LIB" ]; then
    echo "extract-conversational-recommendation.sh: missing matcher library: $_MATCHER_LIB" >&2
    exit 2
fi
# shellcheck source=lib/extract-matchers.sh
. "$_MATCHER_LIB"

S1="$(extract_section_headings)"
S2="$(extract_fenced_blocks)"
S3="$(extract_numbered_action_list)"
S3_5="$(extract_next_prefix_continuations)"
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
append_if_nonempty "$S3_5"
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
