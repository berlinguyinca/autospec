#!/usr/bin/env bash
# scripts/lint-issue.sh — issue-body quality-gate rule engine.
#
# Usage:
#   scripts/lint-issue.sh <body-file>          # exit 0=pass, N=number of findings
#   scripts/lint-issue.sh --json <body-file>   # findings as JSON array
#   scripts/lint-issue.sh --help               # print rules summary
#
# Output (default, on fail): one finding per stderr line, format:
#   <RULE_ID>: <1-line description>
# where RULE_ID is GOAL_VAGUE | GOAL_HEDGE | GOAL_NOT_ONE_SENTENCE
#                | AC_PROSE | AC_SUBJECTIVE | AC_TOO_LONG | AC_EMPTY
#                | SMOKE_MULTI_LINE | SMOKE_PLACEHOLDER | SMOKE_NOT_FENCED
#
# Exit code = number of distinct findings (capped at 64); 0 means pass.

set -eu

HELP_TEXT="Usage: scripts/lint-issue.sh [--json] <body-file>
       scripts/lint-issue.sh --help

Rules enforced (§3 quality contract):

  GOAL_VAGUE            Bare vague verb (improve|enhance|optimize|polish|simplify|refactor|harden)
                        without a concrete object (path, backtick-quoted term, number, UPPER_SNAKE).
  GOAL_HEDGE            Hedging word (should|might|could try|try to) found in Goal section.
  GOAL_NOT_ONE_SENTENCE Goal section must contain exactly one sentence (one terminal . ? or !).
  AC_PROSE              AC line is not a checkbox (must start with '- [ ]').
  AC_SUBJECTIVE         AC item contains subjective adjective (looks|feels|seems|nice|clean|elegant|appropriate).
  AC_TOO_LONG           AC item exceeds 120 characters (excluding '- [ ] ' prefix).
  AC_EMPTY              Acceptance criteria section has no checkbox items.
  SMOKE_MULTI_LINE      Primary smoke test block has more than one non-blank/non-comment line.
  SMOKE_PLACEHOLDER     Primary smoke test block contains ... <TODO> TBD or XXX.
  SMOKE_NOT_FENCED      No fenced code block found under Primary smoke test heading.

Exit code = number of findings (capped at 64). Exit 0 means all rules pass."

# ── argument parsing ──────────────────────────────────────────────────────────

JSON_MODE=0
BODY_FILE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)
            printf '%s\n' "$HELP_TEXT"
            exit 0
            ;;
        --json)
            JSON_MODE=1
            shift
            ;;
        -*)
            printf 'lint-issue.sh: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
        *)
            BODY_FILE="$1"
            shift
            ;;
    esac
done

if [ -z "$BODY_FILE" ]; then
    printf 'lint-issue.sh: missing <body-file> argument\n' >&2
    printf '%s\n' "$HELP_TEXT" >&2
    exit 1
fi

if [ ! -f "$BODY_FILE" ]; then
    printf 'lint-issue.sh: file not found: %s\n' "$BODY_FILE" >&2
    exit 1
fi

# ── section extraction helpers ────────────────────────────────────────────────

# Extract lines between a ## heading and the next ## heading (exclusive).
# Usage: extract_section "## Goal" "$content"
extract_section() {
    local heading="$1"
    local file="$2"
    awk -v h="$heading" '
        $0 == h { in_section=1; next }
        in_section && /^## / { exit }
        in_section { print }
    ' "$file"
}

# Extract lines between a ### subheading and the next ## or ### heading.
extract_subsection() {
    local heading="$1"
    local file="$2"
    awk -v h="$heading" '
        $0 ~ h { in_section=1; next }
        in_section && /^##/ { exit }
        in_section { print }
    ' "$file"
}

# ── findings accumulator ──────────────────────────────────────────────────────

FINDINGS=""

add_finding() {
    local rule_id="$1"
    local desc="$2"
    if [ -z "$FINDINGS" ]; then
        FINDINGS="${rule_id}: ${desc}"
    else
        FINDINGS="${FINDINGS}
${rule_id}: ${desc}"
    fi
}

count_findings() {
    if [ -z "$FINDINGS" ]; then
        printf '0'
    else
        printf '%s\n' "$FINDINGS" | wc -l | tr -d ' '
    fi
}

# ── §3.1 Goal concreteness rules ──────────────────────────────────────────────

check_goal() {
    local goal_content
    goal_content="$(extract_section '## Goal' "$BODY_FILE" | sed '/^$/d')"

    if [ -z "$goal_content" ]; then
        add_finding "GOAL_NOT_ONE_SENTENCE" "Goal section is empty or missing"
        return
    fi

    # Count terminal punctuation (. ? !) to determine sentence count.
    # We collapse the section to a single string and count terminals.
    local full_text
    full_text="$(printf '%s' "$goal_content" | tr '\n' ' ' | sed 's/  */ /g' | sed 's/^ //;s/ $//')"

    # Count sentence-terminal characters: ? or ! anywhere, or . that is followed by
    # whitespace/end-of-string (to skip dots inside .sh, .md, 3.1, etc.)
    local terminal_count
    terminal_count="$(printf '%s' "$full_text" | grep -oE '[?!]|\.[[:space:]]|\.$' | wc -l | tr -d ' ')"

    if [ "$terminal_count" -ne 1 ]; then
        add_finding "GOAL_NOT_ONE_SENTENCE" "Goal section must be exactly one sentence; found ${terminal_count} terminal punctuation mark(s)"
    fi

    # Check for concrete object: path token, backtick-quoted span, number, UPPER_SNAKE label/env-var
    local has_concrete=0
    # path token: /..., or *.ext
    printf '%s' "$full_text" | grep -qE '(/[A-Za-z0-9_./\-]+|\*\.[a-zA-Z]+)' && has_concrete=1
    # backtick-quoted span
    printf '%s' "$full_text" | grep -qE '`[^`]+`' && has_concrete=1
    # number (standalone integer)
    printf '%s' "$full_text" | grep -qE '\b[0-9]+\b' && has_concrete=1
    # UPPER_SNAKE (labels, env vars: at least 2 chars, all caps + underscore)
    printf '%s' "$full_text" | grep -qE '\b[A-Z][A-Z0-9_]{1,}\b' && has_concrete=1

    # Check vague verbs — only fail if no concrete object is present
    if printf '%s' "$full_text" | grep -qiE '\b(improve|enhance|optimize|polish|simplify|refactor|harden)\b'; then
        if [ "$has_concrete" -eq 0 ]; then
            local vague_match
            vague_match="$(printf '%s' "$full_text" | grep -oiE '\b(improve|enhance|optimize|polish|simplify|refactor|harden)\b' | head -1)"
            add_finding "GOAL_VAGUE" "Bare vague verb '${vague_match}' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)"
        fi
    fi

    # Check hedging words — always fail regardless of concrete object
    if printf '%s' "$full_text" | grep -qiE '\b(should|might|could try|try to)\b'; then
        local hedge_match
        hedge_match="$(printf '%s' "$full_text" | grep -oiE '\b(should|might|could try|try to)\b' | head -1)"
        add_finding "GOAL_HEDGE" "Hedging word '${hedge_match}' found in Goal section; state the outcome flatly"
    fi
}

# ── §3.2 AC machine-checkability rules ───────────────────────────────────────

check_ac() {
    local ac_content
    ac_content="$(extract_section '## Acceptance criteria' "$BODY_FILE")"

    # Check for alternative heading forms
    if [ -z "$ac_content" ]; then
        ac_content="$(extract_section '## Acceptance Criteria' "$BODY_FILE")"
    fi

    # Filter to non-blank lines only
    local ac_lines
    ac_lines="$(printf '%s\n' "$ac_content" | sed '/^[[:space:]]*$/d')"

    if [ -z "$ac_lines" ]; then
        add_finding "AC_EMPTY" "Acceptance criteria section has no checkbox items (section missing or empty)"
        return
    fi

    # Check that at least one checkbox item exists
    local checkbox_count
    checkbox_count="$(printf '%s\n' "$ac_lines" | grep -cE '^\s*-\s*\[ \]' || true)"

    if [ "$checkbox_count" -eq 0 ]; then
        add_finding "AC_EMPTY" "Acceptance criteria section has no '- [ ]' checkbox items"
        return
    fi

    # Check each non-blank line
    local line_num=0
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        line_num=$((line_num + 1))

        # Must start with - [ ]
        if ! printf '%s' "$line" | grep -qE '^\s*-\s*\[ \]\s+\S'; then
            add_finding "AC_PROSE" "AC line ${line_num} is not a checkbox ('- [ ]' with content required): $(printf '%s' "$line" | cut -c1-60)"
            continue
        fi

        # Must contain a concrete machine-checkable token
        local has_token=0
        printf '%s' "$line" | grep -qE '(/[A-Za-z0-9_./\-]+|\*\.[a-zA-Z]+)' && has_token=1
        printf '%s' "$line" | grep -qE '`[^`]+`' && has_token=1
        printf '%s' "$line" | grep -qE '\b[0-9]+\b' && has_token=1
        printf '%s' "$line" | grep -qF '\' && has_token=1  # regex literal (backslash present)

        # Subjective words check
        if printf '%s' "$line" | grep -qiE '\b(looks|feels|seems|nice|clean|elegant|appropriate)\b'; then
            local subj_match
            subj_match="$(printf '%s' "$line" | grep -oiE '\b(looks|feels|seems|nice|clean|elegant|appropriate)\b' | head -1)"
            add_finding "AC_SUBJECTIVE" "AC item ${line_num} contains subjective word '${subj_match}': $(printf '%s' "$line" | cut -c1-60)"
        fi

        # Length check: strip "- [ ] " prefix (6 chars) and measure
        local item_body
        item_body="$(printf '%s' "$line" | sed 's/^\s*-\s*\[\s\]\s*//')"
        local item_len
        item_len="${#item_body}"
        if [ "$item_len" -gt 120 ]; then
            add_finding "AC_TOO_LONG" "AC item ${line_num} is ${item_len} chars (max 120): $(printf '%s' "$item_body" | cut -c1-60)..."
        fi

    done <<EOF
$ac_lines
EOF
}

# ── §3.3 Primary smoke test shape rules ──────────────────────────────────────

check_smoke() {
    # Find the Primary smoke test subsection; fall back to ## Verification
    local smoke_content
    smoke_content="$(extract_subsection '### Primary smoke test' "$BODY_FILE")"

    if [ -z "$smoke_content" ]; then
        # Try under ## Verification directly
        smoke_content="$(extract_section '## Verification' "$BODY_FILE")"
    fi

    if [ -z "$smoke_content" ]; then
        # No smoke test section at all — not required to fail if missing
        return
    fi

    # Find first fenced code block
    local in_fence=0
    local fence_lines=""
    local found_fence=0

    while IFS= read -r line; do
        if [ "$in_fence" -eq 0 ] && printf '%s' "$line" | grep -qE '^```'; then
            in_fence=1
            found_fence=1
            continue
        fi
        if [ "$in_fence" -eq 1 ] && printf '%s' "$line" | grep -qE '^```'; then
            in_fence=0
            break
        fi
        if [ "$in_fence" -eq 1 ]; then
            fence_lines="${fence_lines}${line}
"
        fi
    done <<EOF
$smoke_content
EOF

    if [ "$found_fence" -eq 0 ]; then
        add_finding "SMOKE_NOT_FENCED" "No fenced code block found under Primary smoke test heading"
        return
    fi

    # Check for placeholders
    if printf '%s' "$fence_lines" | grep -qE '\.\.\.|<TODO>|\bTBD\b|\bXXX\b'; then
        local placeholder
        placeholder="$(printf '%s' "$fence_lines" | grep -oE '\.\.\.|<TODO>|\bTBD\b|\bXXX\b' | head -1)"
        add_finding "SMOKE_PLACEHOLDER" "Primary smoke test block contains placeholder '${placeholder}'"
    fi

    # Count non-blank, non-comment lines
    local code_line_count
    code_line_count="$(printf '%s' "$fence_lines" | sed '/^[[:space:]]*$/d' | sed '/^[[:space:]]*#/d' | wc -l | tr -d ' ')"

    if [ "$code_line_count" -gt 1 ]; then
        add_finding "SMOKE_MULTI_LINE" "Primary smoke test block has ${code_line_count} non-blank/non-comment lines (must be exactly 1; use '&&' to chain)"
    fi
}

# ── main ──────────────────────────────────────────────────────────────────────

check_goal
check_ac
check_smoke

finding_count="$(count_findings)"

# Cap at 64
if [ "$finding_count" -gt 64 ]; then
    finding_count=64
fi

if [ "$JSON_MODE" -eq 1 ]; then
    # Emit findings as JSON array to stdout
    if [ -z "$FINDINGS" ]; then
        printf '[]\n'
    else
        printf '[\n'
        first=1
        while IFS= read -r finding; do
            [ -z "$finding" ] && continue
            # Split on first ": "
            rule_id="$(printf '%s' "$finding" | sed 's/: .*//')"
            desc="$(printf '%s' "$finding" | sed 's/^[^:]*: //')"
            # JSON-escape desc: escape backslashes then double-quotes
            desc_escaped="$(printf '%s' "$desc" | sed 's/\\/\\\\/g' | sed 's/"/\\"/g')"
            if [ "$first" -eq 1 ]; then
                printf '  {"rule":"%s","description":"%s"}' "$rule_id" "$desc_escaped"
                first=0
            else
                printf ',\n  {"rule":"%s","description":"%s"}' "$rule_id" "$desc_escaped"
            fi
        done <<EOF2
$FINDINGS
EOF2
        printf '\n]\n'
    fi
else
    # Emit findings to stderr, one per line
    if [ -n "$FINDINGS" ]; then
        printf '%s\n' "$FINDINGS" >&2
    fi
fi

exit "$finding_count"
