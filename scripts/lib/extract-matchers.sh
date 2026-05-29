#!/usr/bin/env bash
# scripts/lib/extract-matchers.sh — shared matcher library for
# autospec-continue and autospec-refine harvest paths (issue #707).
#
# Sourced by:
#   - scripts/extract-conversational-recommendation.sh  (PR #702)
#   - scripts/refine-prompt.sh::run_continue_loop()      (PR #678 + #693)
#
# Matcher priority chain (highest -> lowest):
#   1. Section headings:   ## Next steps / What to do next / Remaining work /
#                          Open blockers / Suggestions / Proposed follow-ups
#   2. Fenced blocks:      ```autospec-next / ```next-prompt
#   3. Numbered list with >=50% imperative-verb items
#   3.5. Next-prefix / continuation prefixes (issue #707):
#           Next best slice:, Next best step:, Next slice:, Next step:,
#           Next:, Continue with:, Proceed with:, Move on to:, Move to:,
#           Then:, Up next:, Up next is:, Suggested next:
#         Each match extracts the matched line + following paragraph
#         (up to next blank line or `##` heading).
#   4. Sentences starting with you should / I suggest / I recommend /
#      next step is / the next thing to do is
#
# Every function reads from $MSG (set by the caller) and writes its
# extracted text to stdout.

# -----------------------------------------------------------------------------
# Matcher 1: section headings.
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
# Matcher 2: fenced ```autospec-next / ```next-prompt blocks.
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
# Matcher 3.5: Next-prefix / continuation prefixes (issue #707).
#
# Recognised (case-insensitive, sentence-start):
#   Next best slice:, Next best step:, Next slice:, Next step:, Next:,
#   Continue with:, Proceed with:, Move on to:, Move to:, Then:,
#   Up next:, Up next is:, Suggested next:
#
# Each match extracts the matched line + the following paragraph (up to the
# next blank line or `##` heading).
# -----------------------------------------------------------------------------
extract_next_prefix_continuations() {
    awk '
        BEGIN {
            # Order matters: longer / more specific prefixes first so awk
            # matches "Next best slice:" before "Next:".
            pat = "^[[:space:]]*(next best slice:|next best step:|next slice:|next step:|continue with:|proceed with:|move on to:|move to:|up next is:|up next:|suggested next:|then:|next:)"
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
                    next
                }
                if ($0 ~ /^##[[:space:]]+/) {
                    in_para = 0
                    next
                }
                print
            }
        }
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
