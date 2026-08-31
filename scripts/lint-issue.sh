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
#                | MISSING_SECTION_FILES_TO_READ | MISSING_SECTION_IMPL_OUTLINE
#                | MISSING_SECTION_TESTS | DEPS_MALFORMED
#                | FILES_TOUCHED_MALFORMED | TOO_MANY_FILES
#                | BODY_TOO_LONG | OUTLINE_TOO_LONG
#
# Implementer-load-bearing section + sizing rules (autospec trackers #420/#421):
# the per-issue implementer reads "Files to read first", "Implementation outline",
# and "Tests required"; the monitor parses "Dependencies"; scope comes from
# "Files touched". These rules enforce the prose-only contract deterministically.
#
# Exit code = number of distinct findings (capped at 64); 0 means pass.

set -eu

HELP_TEXT="Usage: scripts/lint-issue.sh [--json] <body-file>
       scripts/lint-issue.sh --help

Rules enforced (§3 quality contract):

  GOAL_VAGUE            Bare vague verb (improve|enhance|optimize|polish|simplify|refactor|harden)
                        without a concrete object (path, backtick-quoted term, number, UPPER_SNAKE).
  GOAL_HEDGE            Hedging word (should|might|could try|try to) found in Goal section.
  GOAL_NOT_ONE_SENTENCE Goal section is empty/missing, is not exactly 1 sentence, or exceeds 30 words.
  AC_PROSE              AC line is not a checkbox (must start with '- [ ]').
  AC_SUBJECTIVE         AC item contains subjective adjective (looks|feels|seems|nice|clean|elegant|appropriate).
  AC_TOO_LONG           AC item exceeds 120 characters (excluding '- [ ] ' prefix).
  AC_EMPTY              Acceptance criteria section has no checkbox items.
  AC_NOT_CHECKABLE      AC item lacks a path, backtick span, integer, or regex token.
  SMOKE_MULTI_LINE      Primary smoke test block does not have exactly one executable line.
  SMOKE_PLACEHOLDER     Primary smoke test block contains ... <TODO> TBD or XXX.
  SMOKE_NOT_FENCED      No fenced code block found under Primary smoke test heading.
  MISSING_SECTION_FILES_TO_READ   Body has no '## Files to read first' heading.
  MISSING_SECTION_IMPL_OUTLINE    Body has no '## Implementation outline' heading.
  MISSING_SECTION_TESTS           Body has no '## Tests required' heading.
  MISSING_SECTION_DEPENDENCIES    Body has no '## Dependencies' heading.
  DEPS_MALFORMED        A '## Dependencies' section exists but a content line is neither
                        'Depends on issue #N' nor exactly 'none'.
  FILES_TOUCHED_MALFORMED A non-blank '## Files touched' entry is not exactly one
                        safe repo-relative file or trailing-slash directory.
  TOO_MANY_FILES        A '## Files touched' section lists more than 3 file paths.
  BODY_TOO_LONG         Authored body exceeds 400 words (the five ui-feature
    sections and marker-bounded generated classification/shared-contract/quality
    metadata are excluded from this count).
  OUTLINE_TOO_LONG      '## Implementation outline' section exceeds 30 non-blank lines.
  UI_SECTIONS_INCOMPLETE A UI feature (a '<!-- ui-feature -->' marker or any
    one of the five required sections present) is missing one or more of:
    Design reference, Interaction states, UX flows, Motion & feedback,
    Device & viewport.

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

STDIN_TMP=""
if [ "$BODY_FILE" = "-" ] || [ "$BODY_FILE" = "/dev/stdin" ]; then
    STDIN_TMP="$(mktemp)"
    cat > "$STDIN_TMP"
    BODY_FILE="$STDIN_TMP"
    trap 'rm -f "$STDIN_TMP"' EXIT
elif [ ! -f "$BODY_FILE" ]; then
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

# Strip the five ui-feature sections (heading line + body, up to but excluding
# the next '## ' heading) from a file. Used by check_body_size so the ≤400-word
# cap (§L1a of docs/superpowers/specs/2026-08-04-autospec-web-ui-design.md)
# excludes required UI sections rather than counting them against classified
# children. Uses exact string membership (not regex) so literal '&' in
# 'Motion & feedback' needs no escaping.
strip_ui_sections() {
    awk '
        BEGIN {
            headings["## Design reference"] = 1
            headings["## Interaction states"] = 1
            headings["## UX flows"] = 1
            headings["## Motion & feedback"] = 1
            headings["## Device & viewport"] = 1
        }
        {
            # Trailing whitespace is trimmed before the lookup, because the
            # check_ui_sections greps accept it when deciding the section is
            # present. The two must agree, or a heading written with markdown’s
            # two-space hard line break counts against the cap it is exempt from.
            line = $0
            sub(/[[:space:]]+$/, "", line)
            if (line in headings) { skip = 1; next }
            # A generated block ends the UI section just as a new heading does.
            # Without this, the skip swallows the block'"'"'s opening marker (it is not
            # a "## " line), strip_generated_metadata then sees an unmatched end
            # marker, declines to strip, and the whole block is charged to the
            # authored count -- the leak the exemption exists to prevent.
            if (skip && line ~ /^<!-- autospec-[a-z-]+:begin -->$/) { skip = 0 }
            if (skip && substr($0, 1, 3) == "## ") { skip = 0 }
            if (skip) { next }
            print
        }
    '
}

# Strip marker-bounded metadata appended after the authored child body has
# already passed its 400-word budget.
strip_generated_metadata() {
    awk '
        {
            lines[NR] = $0
            if ($0 ~ /<!-- autospec-classify:begin -->/) classify_begin = NR
            if ($0 ~ /<!-- autospec-classify:end -->/) classify_end = NR
            if ($0 ~ /<!-- autospec-shared-contracts:begin -->/) shared_begin = NR
            if ($0 ~ /<!-- autospec-shared-contracts:end -->/) shared_end = NR
            if ($0 ~ /<!-- autospec-quality:begin -->/) quality_begin = NR
            if ($0 ~ /<!-- autospec-quality:end -->/) quality_end = NR
        }
        END {
            for (line = 1; line <= NR; line++) {
                in_classify = classify_begin && classify_end && classify_begin < classify_end \
                    && line >= classify_begin && line <= classify_end
                in_shared = shared_begin && shared_end && shared_begin < shared_end \
                    && line >= shared_begin && line <= shared_end
                in_quality = quality_begin && quality_end && quality_begin < quality_end \
                    && line >= quality_begin && line <= quality_end
                if (!in_classify && !in_shared && !in_quality) print lines[line]
            }
        }
    '
}

# strip_ui_sections must run first so that authored prose following a generated
# block is still counted: the block is what ends the UI section, and removing it
# beforehand would let the UI skip run on past it. This is safe only because
# strip_ui_sections terminates its skip on a generated begin marker; without that
# it would eat the marker and disable the exemption entirely.
strip_non_authored_sections() {
    strip_ui_sections < "$1" | strip_generated_metadata
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

    # A Goal is exactly one sentence and up to 30 words. Count terminal
    # characters: ? or ! anywhere, or . that is
    # followed by whitespace/end-of-string (to skip dots inside .sh, .md, 3.1, etc.)
    local terminal_count
    terminal_count="$(printf '%s' "$full_text" | grep -oE '[?!]|\.[[:space:]]|\.$' | wc -l | tr -d ' ')"

    local word_count
    word_count="$(printf '%s' "$full_text" | wc -w | tr -d ' ')"

    if [ "$terminal_count" -ne 1 ] || [ "$word_count" -gt 30 ]; then
        add_finding "GOAL_NOT_ONE_SENTENCE" "Goal must be exactly 1 sentence and at most 30 words; found ${terminal_count} sentence(s) and ${word_count} word(s)"
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
        if [ "$has_token" -eq 0 ]; then
            add_finding "AC_NOT_CHECKABLE" "AC item ${line_num} lacks a path, backtick span, integer, or regex token"
        fi

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
        add_finding "SMOKE_NOT_FENCED" "No Primary smoke test section with a fenced code block"
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

    if [ "$code_line_count" -ne 1 ]; then
        add_finding "SMOKE_MULTI_LINE" "Primary smoke test block has ${code_line_count} non-blank/non-comment lines (must be exactly 1; use '&&' to chain)"
    fi
}

# ── implementer-load-bearing section presence + dependency format ─────────────
# (autospec trackers #420/#421 — the per-issue implementer reads these sections)

check_sections() {
    # Required headings the implementer loads. extract_section returns empty when
    # the heading is absent; a present-but-empty section still satisfies presence
    # (other rules cover emptiness), so test the heading line directly.
    if ! grep -qE '^## Files to read first[[:space:]]*$' "$BODY_FILE"; then
        add_finding "MISSING_SECTION_FILES_TO_READ" "Body has no '## Files to read first' heading (implementer reads it)"
    fi
    if ! grep -qE '^## Implementation outline[[:space:]]*$' "$BODY_FILE"; then
        add_finding "MISSING_SECTION_IMPL_OUTLINE" "Body has no '## Implementation outline' heading (implementer reads it)"
    fi
    if ! grep -qE '^## Tests required[[:space:]]*$' "$BODY_FILE"; then
        add_finding "MISSING_SECTION_TESTS" "Body has no '## Tests required' heading (implementer reads it)"
    fi

    # Dependencies are required. Every non-blank content line must be
    # 'Depends on issue #N' or exactly 'none'.
    if ! grep -qE '^## Dependencies[[:space:]]*$' "$BODY_FILE"; then
        add_finding "MISSING_SECTION_DEPENDENCIES" "Body has no '## Dependencies' heading"
    else
        local deps_content
        deps_content="$(extract_section '## Dependencies' "$BODY_FILE" | sed '/^[[:space:]]*$/d')"
        local bad_dep=""
        while IFS= read -r line; do
            [ -z "$line" ] && continue
            if ! printf '%s' "$line" | grep -qE '^Depends on issue #[0-9]+$' \
                && [ "$line" != "none" ]; then
                bad_dep="$line"
                break
            fi
        done <<EOF
$deps_content
EOF
        if [ -n "$bad_dep" ]; then
            add_finding "DEPS_MALFORMED" "Dependencies line must be 'Depends on issue #N' or 'none': $(printf '%s' "$bad_dep" | cut -c1-60)"
        fi
    fi
}

# ── scope cap: ## Files touched must list at most 3 paths ──────────────────────

check_files_touched() {
    grep -qE '^## Files touched[[:space:]]*$' "$BODY_FILE" || return 0
    local content
    content="$(extract_section '## Files touched' "$BODY_FILE" | sed '/^[[:space:]]*$/d')"
    # Count DISTINCT logical file units, applying the trio atomic-unit
    # exemption (trio-derivation Phase 2 child D). With derive-trio.sh shipped,
    # a trio edit is one logical change — "edit SKILL.md → derive-trio
    # --in-place → gen-skill-goldens" in one commit — so:
    #   - A skill trio's three members
    #     skills/<x>/{SKILL.md,codex/prompt.md,opencode/agent.md} collapse to a
    #     single `skills/<x>/<trio>` unit.
    #   - Derived tests/fixtures/skill-goldens/*.sha256 files are mechanically
    #     regenerated, so they are excluded from the count entirely.
    # Mirrors scripts/sizing-check.sh's outline-files exemption.
    local units=""
    local raw
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        # Strip one optional bullet and one optional wrapping backtick pair.
        raw="$(printf '%s' "$line" \
            | sed -E 's/^[[:space:]]*-[[:space:]]+//; s/^[[:space:]]+//; s/[[:space:]]+$//')"
        case "$raw" in
            \`*\`) raw="${raw#\`}"; raw="${raw%\`}" ;;
            *\`*)
                add_finding "FILES_TOUCHED_MALFORMED" "Files touched entry must be one safe repo-relative file or trailing-slash directory: $(printf '%s' "$line" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
                continue
                ;;
        esac
        local path_body="${raw%/}" segment invalid=0
        case "$raw" in
            ''|'.'|'/'|/*|*//* ) invalid=1 ;;
        esac
        printf '%s' "$raw" | grep -qE '^[A-Za-z0-9_./-]+$' || invalid=1
        local old_ifs="$IFS"
        IFS='/'
        for segment in $path_body; do
            case "$segment" in ''|'.'|'..') invalid=1 ;; esac
        done
        IFS="$old_ifs"
        if [ "$invalid" -eq 1 ]; then
            add_finding "FILES_TOUCHED_MALFORMED" "Files touched entry must be one safe repo-relative file or trailing-slash directory: $(printf '%s' "$line" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
            continue
        fi
        # Exclusion: derived skill-golden hashes never count.
        case "$raw" in
            tests/fixtures/skill-goldens/*.sha256) continue ;;
        esac
        # Collapse trio members to a single per-skill logical unit.
        case "$raw" in
            skills/*/SKILL.md|skills/*/codex/prompt.md|skills/*/opencode/agent.md)
                local skill_name
                skill_name="$(printf '%s' "$raw" | awk -F/ '{print $2}')"
                raw="skills/${skill_name}/<trio>"
                ;;
        esac
        units="${units}${raw}
"
    done <<EOF
$content
EOF
    local path_count
    path_count="$(printf '%s' "$units" | sed '/^[[:space:]]*$/d' | sort -u | awk 'NF { count++ } END { print count + 0 }')"
    if [ "$path_count" -gt 3 ]; then
        add_finding "TOO_MANY_FILES" "Files touched lists ${path_count} logical units (max 3; trio members + derived goldens count as one); split the issue to stay small-LLM-sized"
    fi
}

# ── whole-body word budget ────────────────────────────────────────────────────

check_body_size() {
    local body_words
    body_words="$(strip_non_authored_sections "$BODY_FILE" | wc -w | tr -d ' ')"
    if [ "$body_words" -gt 400 ]; then
        add_finding "BODY_TOO_LONG" "Body is ${body_words} words (max 400); a small-LLM implementer cannot hold an over-long issue"
    fi
}

# ── implementation outline line budget ────────────────────────────────────────

check_outline_size() {
    grep -qE '^## Implementation outline[[:space:]]*$' "$BODY_FILE" || return 0
    local outline_lines
    outline_lines="$(extract_section '## Implementation outline' "$BODY_FILE" | sed '/^[[:space:]]*$/d' | wc -l | tr -d ' ')"
    if [ "$outline_lines" -gt 30 ]; then
        add_finding "OUTLINE_TOO_LONG" "Implementation outline has ${outline_lines} non-blank lines (max 30); tighten or split"
    fi
}

# UI-feature decomposition: when an issue is a UI/frontend feature it must carry
# the full set of UI sections so a small-LLM implementer (and the visual-fidelity
# QA loop) have design + behavior to work against. Detection is opt-in: a
# `<!-- ui-feature -->` marker OR the presence of any one UI section flags the
# issue as UI; then ALL of Design reference + Interaction states + UX flows +
# Motion & feedback + Device & viewport are required (enforced as a coherent
# group — no false positives on non-UI issues).
check_ui_sections() {
    local has_marker=0 has_dr=0 has_is=0 has_ux=0 has_mf=0 has_dv=0
    grep -qE '<!--[[:space:]]*ui-feature[[:space:]]*-->' "$BODY_FILE" && has_marker=1
    grep -qE '^## Design reference[[:space:]]*$' "$BODY_FILE" && has_dr=1
    grep -qE '^## Interaction states[[:space:]]*$' "$BODY_FILE" && has_is=1
    grep -qE '^## UX flows[[:space:]]*$' "$BODY_FILE" && has_ux=1
    grep -qE '^## Motion & feedback[[:space:]]*$' "$BODY_FILE" && has_mf=1
    grep -qE '^## Device & viewport[[:space:]]*$' "$BODY_FILE" && has_dv=1
    if [ "$has_marker" = 1 ] || [ "$has_dr" = 1 ] || [ "$has_is" = 1 ] || [ "$has_ux" = 1 ] \
        || [ "$has_mf" = 1 ] || [ "$has_dv" = 1 ]; then
        local missing=""
        [ "$has_dr" = 1 ] || missing="$missing '## Design reference'"
        [ "$has_is" = 1 ] || missing="$missing '## Interaction states'"
        [ "$has_ux" = 1 ] || missing="$missing '## UX flows'"
        [ "$has_mf" = 1 ] || missing="$missing '## Motion & feedback'"
        [ "$has_dv" = 1 ] || missing="$missing '## Device & viewport'"
        if [ -n "$missing" ]; then
            add_finding "UI_SECTIONS_INCOMPLETE" "UI feature detected; missing required section(s):${missing} (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)"
        fi
    fi
}

# ── main ──────────────────────────────────────────────────────────────────────

check_goal
check_ac
check_smoke
check_sections
check_files_touched
check_body_size
check_outline_size
check_ui_sections

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
