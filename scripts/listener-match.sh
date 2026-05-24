#!/usr/bin/env bash
# scripts/listener-match.sh — classify a chat phrase as an autospec-listen
# trigger.
#
# Two modes:
#
#   Word mode (default, back-compat):
#     scripts/listener-match.sh "<phrase>"           # phrase as $1
#     echo "<phrase>" | scripts/listener-match.sh    # phrase on stdin
#   Prints exactly one of: issue, spec, none. Always exits 0.
#
#   Classify mode (keyword routing, issue #537):
#     scripts/listener-match.sh --classify "<phrase>"
#     echo "<phrase>" | scripts/listener-match.sh --classify
#   Prints JSON
#     {"match":bool,"skill":<skill|null>,"trigger":<verb|null>,
#      "intent":"imperative|incidental|none","confidence":0-1}
#   on stdout. A non-match is {"match":false,...}. Always exits 0.
#
# The word-mode trigger phrases are sourced from
# `skills/autospec-listen/references/trigger-keywords.md` so the script and
# the documentation can never drift. Matching is case-insensitive and
# word-boundary anchored. Bare nouns ("issue", "spec", "ticket") are NOT
# triggers.
#
# Classify mode adds a verb->skill map (D3) and an imperative-intent gate
# (D4) biased to false-negatives: prefer no route over a misfire (see
# feedback_autospec_design_prefs + feedback_omc_autopilot_misfire).

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

# ── Classify mode (keyword routing, issue #537) ────────────────────────────────
#
# Emit the classifier JSON object. All fields always present.
emit_classify_json() {
    # $1=match(true|false) $2=skill $3=trigger $4=intent $5=confidence
    match="$1"; skill="$2"; trigger="$3"; intent="$4"; conf="$5"
    skill_json="null"; [ -n "$skill" ] && skill_json="\"$skill\""
    trigger_json="null"; [ -n "$trigger" ] && trigger_json="\"$trigger\""
    printf '{"match":%s,"skill":%s,"trigger":%s,"intent":"%s","confidence":%s}\n' \
        "$match" "$skill_json" "$trigger_json" "$intent" "$conf"
}

# Whole-word match of a single token in the (already lower-cased) needle.
# Word boundaries are non-[a-z0-9_-] characters or string ends.
has_word() {
    needle="$1"; word="$2"
    printf '%s' "$needle" | awk -v p="$word" '
        BEGIN { found = 0 }
        {
            s = $0; pl = length(p); sl = length(s)
            for (i = 1; i <= sl - pl + 1; i++) {
                if (substr(s, i, pl) == p) {
                    if (i == 1) { left_ok = 1 }
                    else { lc = substr(s, i-1, 1); left_ok = (lc !~ /[A-Za-z0-9_-]/) }
                    end_pos = i + pl
                    if (end_pos > sl) { right_ok = 1 }
                    else { rc = substr(s, end_pos, 1); right_ok = (rc !~ /[A-Za-z0-9_-]/) }
                    if (left_ok && right_ok) { found = 1; exit }
                }
            }
        }
        END { exit found ? 0 : 1 }'
}

# Intent gate (D4). Returns 0 (imperative — route) or 1 (incidental — suppress).
# Biased to false-negatives: any suppressor wins.
is_imperative() {
    text="$1"   # lower-cased

    # Suppressor 1: question. A trailing "?" or an interrogative lead-in.
    case "$text" in
        *\?*) return 1 ;;
    esac
    for q in should could would shall "do we" "do you" "can we" "can you" \
             "should we" "should i" "could we" "could you" "would you" \
             why what how when "are we" "is it" "are you"; do
        case "$text" in
            "$q "*|"$q") return 1 ;;
        esac
    done

    # Suppressor 2: negation anywhere.
    for neg in "don't" "do not" "doesn't" "does not" "didn't" "did not" \
               "won't" "will not" "wouldn't" "shouldn't" "can't" "cannot" \
               "no need" "not yet" "never" "instead of" "rather than"; do
        if has_word "$text" "$neg" 2>/dev/null; then return 1; fi
        case "$text" in
            *"$neg"*) return 1 ;;
        esac
    done
    # Bare "not" as its own word.
    if has_word "$text" "not"; then return 1; fi

    # Suppressor 3: past tense / already-done.
    for past in already reviewed implemented designed built shipped \
                created filed wrote written reviewing done finished completed; do
        if has_word "$text" "$past"; then return 1; fi
    done
    # "have/has/had <verb>" and "was/were" markers.
    for pmark in "have " "has " "had " "was " "were " "i've " "we've "; do
        case "$text" in
            *"$pmark"*) return 1 ;;
        esac
    done

    # Suppressor 4: descriptive use (verb as a noun behind an article, or a
    # copular description). "the design ...", "this review ...", "a spec ...".
    for art in "the design" "the review" "the spec" "the build" "this design" \
               "this review" "this spec" "that design" "a design" "the implementation"; do
        case "$text" in
            *"$art"*) return 1 ;;
        esac
    done

    return 0
}

# Classify a phrase into the verb→skill map (D3), gated by intent (D4).
classify_phrase() {
    text_lc="$(to_lower "$1")"

    # Back-compat: explicit "file an issue" / "write a spec" phrases are
    # already imperative triggers and bypass the verb intent-gate (they route
    # to autospec-define, the listen skill's define handoff).
    if match_against_list "$text_lc" <<EOF
$(parse_trigger_md "Issue triggers")
EOF
    then
        emit_classify_json true autospec-define issue imperative 0.6
        return 0
    fi
    if match_against_list "$text_lc" <<EOF
$(parse_trigger_md "Spec triggers")
EOF
    then
        emit_classify_json true autospec-define spec imperative 0.6
        return 0
    fi

    # Verb → skill map (D3). Order matters: most specific first.
    skill=""; trigger=""
    if has_word "$text_lc" "autospec"; then
        skill="autospec"; trigger="autospec"
    elif has_word "$text_lc" "implement" || has_word "$text_lc" "build" \
        || has_word "$text_lc" "ship"; then
        skill="autospec-run"
        if has_word "$text_lc" "implement"; then trigger="implement"
        elif has_word "$text_lc" "build"; then trigger="build"
        else trigger="ship"; fi
    elif has_word "$text_lc" "design" || has_word "$text_lc" "redesign" \
        || has_word "$text_lc" "new feature" \
        || (has_word "$text_lc" "spec" && ! has_word "$text_lc" "specification"); then
        skill="autospec-define"
        if has_word "$text_lc" "design" || has_word "$text_lc" "redesign"; then
            trigger="design"
        elif has_word "$text_lc" "new feature"; then trigger="new feature"
        else trigger="spec"; fi
    elif has_word "$text_lc" "review"; then
        skill="autospec-review"; trigger="review"
    fi

    # No verb matched (and no back-compat phrase above) — non-match.
    if [ -z "$skill" ]; then
        emit_classify_json false "" "" none 0
        return 0
    fi

    # A verb matched — apply the intent gate.
    if is_imperative "$text_lc"; then
        emit_classify_json true "$skill" "$trigger" imperative 0.8
    else
        emit_classify_json false "$skill" "$trigger" incidental 0.2
    fi
}

main() {
    mode="word"
    if [ "${1:-}" = "--classify" ]; then
        mode="classify"
        shift
    fi

    if [ "$#" -ge 1 ]; then
        candidate="$1"
    else
        # Read all of stdin into one buffer so multi-line input still works.
        candidate="$(cat)"
    fi

    if [ "$mode" = "classify" ]; then
        classify_phrase "$candidate"
    else
        match_phrase "$candidate"
    fi
}

main "$@"
