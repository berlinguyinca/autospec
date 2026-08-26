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

# Test whether $1 (lower-cased) contains a trigger phrase from the list on
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
    # $1=match(true|false) $2=skill $3=trigger $4=intent $5=confidence $6=autonomous(true|false, optional)
    # $7=chain(skill|empty) $8=gate(string|empty)
    match="$1"; skill="$2"; trigger="$3"; intent="$4"; conf="$5"; autonomous="${6:-false}"
    chain="${7:-}"; gate="${8:-}"
    skill_json="null"; [ -n "$skill" ] && skill_json="\"$skill\""
    trigger_json="null"; [ -n "$trigger" ] && trigger_json="\"$trigger\""
    chain_json="null"; [ -n "$chain" ] && chain_json="\"$chain\""
    gate_json="null"; [ -n "$gate" ] && gate_json="\"$gate\""
    json_string() {
        if [ -n "$1" ]; then
            printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
        else
            printf 'null'
        fi
    }
    plan_path_json="$(json_string "${AUTOSPEC_LISTENER_PLAN_PATH:-}")"
    source_spec_path_json="$(json_string "${AUTOSPEC_LISTENER_SOURCE_SPEC_PATH:-}")"
    repo_json="$(json_string "${AUTOSPEC_LISTENER_REPO:-}")"
    branch_json="$(json_string "${AUTOSPEC_LISTENER_BRANCH:-}")"
    base_json="$(json_string "${AUTOSPEC_LISTENER_BASE:-}")"
    ts_json="$(json_string "${AUTOSPEC_LISTENER_PLAN_TS:-}")"
    printf '{"match":%s,"skill":%s,"trigger":%s,"intent":"%s","confidence":%s,"autonomous":%s,"chain":%s,"gate":%s,"plan_path":%s,"source_spec_path":%s,"repo":%s,"branch":%s,"base":%s,"plan_ts":%s}\n' \
        "$match" "$skill_json" "$trigger_json" "$intent" "$conf" "$autonomous" "$chain_json" "$gate_json" \
        "$plan_path_json" "$source_spec_path_json" "$repo_json" "$branch_json" "$base_json" "$ts_json"
}

# Autonomous-phrase detection (issue #662): scan the lowercased text for
# operator phrasing that maps to --autonomous routing. Returns 0 (autonomous)
# or 1 (not autonomous).
is_autonomous_phrase() {
    needle="$1"
    # Patterns: "fix X automatically", "just do it", "no confirmation",
    # "non-interactive", "go autonomous", "autonomous mode", "run autonomously",
    # "do it automatically", "without asking".
    printf '%s' "$needle" | grep -qE 'automatically|just do it|no confirmation|non-interactive|non interactive|go autonomous|autonomous mode|run autonomously|without asking|without confirmation' \
        && return 0
    return 1
}

# One-turn listener escape for routes that were offered with
# "say plain to opt out".
is_listener_escape() {
    text="$1"
    case "$text" in
        plain|no|"no workflow"|"just chat"|"do not implement"|stop|cancel) return 0 ;;
    esac
    return 1
}

# Post-approval autospec handoff state is supplied by the listener/wrapper from
# recent conversation memory. The classifier stays deterministic and stateless;
# these env vars make the handoff decision testable without a live harness.
has_post_approval_state() {
    [ -n "${AUTOSPEC_LISTENER_APPROVED_SPEC_PATH:-}" ] \
        || [ -n "${AUTOSPEC_LISTENER_PLAN_PATH:-}" ] \
        || [ "${AUTOSPEC_LISTENER_AUTOSPEC_REQUESTED:-0}" = "1" ] \
        || [ "${AUTOSPEC_LISTENER_AUTONOMOUS_REQUESTED:-0}" = "1" ]
}

has_plan_exit_ready_state() {
    { [ "${AUTOSPEC_LISTENER_PLAN_EXIT_READY:-0}" = "1" ] \
        || [ "${AUTOSPEC_LISTENER_PLAN_EXIT_READY:-}" = "true" ]; } \
        && { [ -n "${AUTOSPEC_LISTENER_PLAN_PATH:-}" ] \
            || [ -n "${AUTOSPEC_LISTENER_PLAN_ARTIFACT:-}" ]; }
}

is_blocked_plan_exit() {
    text="$1"
    case "${AUTOSPEC_LISTENER_PLAN_EXIT_BLOCKED:-0}" in
        1|true|TRUE|yes|YES) return 0 ;;
    esac
    case "$text" in
        *blocked*|*"missing requirement"*|*"missing requirements"*|*ambiguous*|*ambiguity*)
            return 0
            ;;
    esac
    return 1
}

requires_destructive_approval() {
    text="$1"
    case "${AUTOSPEC_LISTENER_PLAN_DESTRUCTIVE:-0}" in
        1|true|TRUE|yes|YES) return 0 ;;
    esac
    case "$text" in
        *destructive*|*production*|*credential*|*credentials*|*irreversible*|*"requires approval"*|*"explicit approval"*)
            return 0
            ;;
    esac
    return 1
}

is_approval_phrase() {
    text="$1"
    case "$text" in
        "looks good"|"looks good to me"|approved|approve|yes|yep|ok|okay|go|continue|proceed|"go ahead"|"do it"|"ship it")
            return 0
            ;;
    esac
    return 1
}

is_execution_choice_prompt() {
    text="$1"
    case "$text" in
        *"subagent-driven"*|*"inline execution"*|*"choose execution"*|*"choose how to execute"*|*"execution choice"*|*"how to execute it"*)
            return 0
            ;;
    esac
    return 1
}

is_conceptual_plan_question() {
    text="$1"
    for q in why what how when "should we" "could we" "would it" "what are" "what is"; do
        case "$text" in
            "$q "*|"$q"|*"$q "*) return 0 ;;
        esac
    done
    return 1
}

has_open_auto_implement_hint() {
    case "${AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN:-0}" in
        ''|0|false|False|FALSE) return 1 ;;
        *) return 0 ;;
    esac
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
# Biased to false-negatives: each suppressor wins.
is_imperative() {
    text="$1"   # lower-cased

    # Suppressor 1: question. A trailing "?" or an interrogative lead-in.
    case "$text" in
        *\?*) return 1 ;;
    esac
    for q in should could would shall "do we" "do you" "can we" "can you" \
             "should we" "should i" "could we" "could you" "would you" \
             "did you" "did we" "did i" "does it" "do they" "do i" \
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
    # Descriptive references to the system should not route merely because the
    # word "autospec" appears.
    case "$text" in
        *"the autospec"*|*"this autospec"*|*"that autospec"*|*"an autospec"*|*"autospec plan"*|*"autospec spec"*)
            return 1
            ;;
    esac

    # Suppressor 5: explore/discover read-and-understand idioms (D4, issue #909).
    # Reinforces the question/negation/past groups for the explore verb. These
    # are narrow read-intent phrasings — NOT a blanket "explore the" suppressor,
    # which would wrongly kill "explore the repo for improvements" (a CLAIM
    # phrase). The build/ship co-occurrence gate in classify_phrase() is the
    # primary guard; this is belt-and-suspenders for descriptive explore use.
    for rd in "explore the codebase" "explore this" "explore the data" \
              "explore the file" "explore the files" "explore options" \
              "let me explore" "exploratory" "the exploration"; do
        case "$text" in
            *"$rd"*) return 1 ;;
        esac
    done

    return 0
}

# Classify a phrase into the verb→skill map (D3), gated by intent (D4).
classify_phrase() {
    text_lc="$(to_lower "$1")"

    # Autonomous-phrase detection (issue #662). If the text contains an
    # autonomous-routing phrase, mark autonomous=true and proceed; if no verb
    # matches downstream, default to /autospec autonomous-end-to-end.
    is_auto="false"
    if is_autonomous_phrase "$text_lc"; then
        is_auto="true"
    fi

    # Completed Plan-mode exit handoff (issue #1462). Harness/workflow state
    # can say a saved implementation plan is ready even when the visible text is
    # only a generic execution-choice prompt. Route that directly into autospec
    # unless the same turn is an opt-out, blocked plan, conceptual question, or
    # destructive/credential-gated approval boundary.
    if has_plan_exit_ready_state; then
        if is_listener_escape "$text_lc" \
            || is_blocked_plan_exit "$text_lc" \
            || requires_destructive_approval "$text_lc"; then
            emit_classify_json false "" "" none 0 false "" ""
            return 0
        fi
        trigger="plan_exit_ready"
        if is_approval_phrase "$text_lc"; then
            trigger="plan_approved_ready"
        elif is_conceptual_plan_question "$text_lc" && ! is_execution_choice_prompt "$text_lc"; then
            emit_classify_json false "" "" none 0 false "" ""
            return 0
        fi
        if has_open_auto_implement_hint; then
            emit_classify_json true autospec-run "$trigger" imperative 0.9 false "" ""
        else
            emit_classify_json true autospec "$trigger" imperative 0.9 true "" ""
        fi
        return 0
    fi

    # Post-approval execution-ready handoff (issue #1461). When a wrapper has
    # recorded an approved spec/plan or prior autospec/autonomous request in the
    # thread, plain approval should route directly into autospec execution
    # instead of asking the user to choose an execution mode.
    if has_post_approval_state && is_approval_phrase "$text_lc"; then
        if is_listener_escape "$text_lc" || ! is_imperative "$text_lc"; then
            emit_classify_json false "" "" none 0 false "" ""
            return 0
        fi
        if has_open_auto_implement_hint; then
            emit_classify_json true autospec-run post_approval_execution_ready imperative 0.85 false "" ""
        else
            emit_classify_json true autospec post_approval_execution_ready imperative 0.85 true "" ""
        fi
        return 0
    fi

    if is_listener_escape "$text_lc"; then
        emit_classify_json false "" "" none 0 false "" ""
        return 0
    fi

    # Back-compat: explicit "file an issue" / "write a spec" phrases are
    # already imperative triggers and bypass the verb intent-gate (they route
    # to autospec-define, the listen skill's define handoff).
    if match_against_list "$text_lc" <<EOF
$(parse_trigger_md "Issue triggers")
EOF
    then
        emit_classify_json true autospec-define issue imperative 0.6 "$is_auto"
        return 0
    fi
    if match_against_list "$text_lc" <<EOF
$(parse_trigger_md "Spec triggers")
EOF
    then
        emit_classify_json true autospec-define spec imperative 0.6 "$is_auto"
        return 0
    fi

    # Verb → skill map (D3). Order matters: most specific first.
    skill=""; trigger=""; chain=""; gate=""

    # ── 1. Combined refine+run (most specific) ──────────────────────────────
    # Phrases that mean "refine then also run the pipeline". Matched before
    # bare refine/run so the combined semantics win.
    _is_combined=0
    for _combo in "refine and run" "refine then run" "refine & run" \
                  "optimize and run" "optimize then run" "polish and run" \
                  "tune up" "tune it up" "tune things up"; do
        case "$text_lc" in
            *"$_combo"*) _is_combined=1; break ;;
        esac
    done
    if [ "$_is_combined" -eq 1 ]; then
        if is_imperative "$text_lc"; then
            emit_classify_json true autospec-refine refine-and-run imperative 0.8 "$is_auto" autospec-run ""
        else
            emit_classify_json false autospec-refine refine-and-run incidental 0.2 false "" ""
        fi
        return 0
    fi

    # ── 2. Run — scoped phrases (unconditional, no gate) ────────────────────
    # Explicit "run <autospec-object>" phrases that unambiguously mean the
    # autospec-run pipeline; no context gate required.
    _is_run_scoped=0
    for _scoped in "run autospec" "run the loop" "run the queue" "run the batch" \
                   "run the pipeline" "run it" "run them" "drain the queue"; do
        case "$text_lc" in
            *"$_scoped"*) _is_run_scoped=1; break ;;
        esac
    done
    if [ "$_is_run_scoped" -eq 1 ]; then
        if is_imperative "$text_lc"; then
            emit_classify_json true autospec-run run imperative 0.8 "$is_auto" "" ""
        else
            emit_classify_json false autospec-run run incidental 0.2 false "" ""
        fi
        return 0
    fi

    # ── 3. Run — bare (context-gated) ───────────────────────────────────────
    # A bare "run" that is NOT followed by a non-autospec object. Requires the
    # auto-implement-open gate: only route if open auto-implement issues exist.
    # Gerund "running" and suppressed objects fall through.
    if has_word "$text_lc" "run"; then
        # Gerund suppressor: "running" as a word.
        _is_gerund=0
        case "$text_lc" in
            *"running"*) _is_gerund=1 ;;
        esac

        # Suppressed objects: "run <non-autospec-thing>".
        _is_suppressed=0
        for _obj in "run the test" "run the tests" "run the suite" \
                    "run the build" "run the builds" \
                    "run the script" "run the scripts" \
                    "run the server" "run the dev server" \
                    "run the app" "run lint" "run the linter" \
                    "run ci" "run the ci" \
                    "run the benchmark" "run the benchmarks" \
                    "run the migration" "run the migrations" \
                    "run the demo" "run the command" "run the commands"; do
            case "$text_lc" in
                *"$_obj"*) _is_suppressed=1; break ;;
            esac
        done

        # Suppressed "run the <non-autospec-object>" (e.g. "run the build",
        # "run the tests"): the user means that object, not autospec. Emit a
        # clean no-match and return so an embedded build/ship/implement word in
        # the object cannot re-trigger autospec-run in the verb map below.
        if [ "$_is_suppressed" -eq 1 ]; then
            emit_classify_json false "" "" none 0 false "" ""
            return 0
        fi

        if [ "$_is_gerund" -eq 0 ]; then
            if is_imperative "$text_lc"; then
                emit_classify_json true autospec-run run imperative 0.7 "$is_auto" "" auto-implement-open
            else
                emit_classify_json false autospec-run run incidental 0.2 false "" ""
            fi
            return 0
        fi
        # Gerund ("running"): fall through to remaining verbs.
    fi

    # ── Explore / discover — research-and-ship intent (D3, issue #909) ───────
    # Route to /autospec-explore ONLY when an explore/discover verb co-occurs
    # with a COORDINATED build/ship/feature action ("explore and ship",
    # "discover features to build", "explore the repo for improvements"). A bare
    # "explore <noun>" — including "explore the feature flags" / "explore the
    # build dir", where the build/feature word is an incidental noun with no
    # coordinating action connector — must NOT route. That is the critical
    # safety property guarding against auto-launching a perpetual PR-shipping
    # loop.
    # Placed BEFORE implement/build/ship so the more-specific explore semantics
    # win over the embedded build/ship/implement verb. Emitted explicitly (like
    # the bare-run gate exemplar above) so it carries gate=explore-confirm,
    # consumed byte-for-byte by the autospec-listen trio (issue #910).
    # `discover` is an alias verb routing identically (same skill/trigger/gate).
    _has_explore=0
    if has_word "$text_lc" "explore" || has_word "$text_lc" "discover" \
        || has_word "$text_lc" "exploring" || has_word "$text_lc" "discovering"; then
        _has_explore=1
    fi
    # Build intent requires an explicit ACTION CONNECTOR ("and/to/for") joined
    # to a build/ship/feature word — NOT mere co-occurrence. This is the safety
    # discriminator: "explore the feature flags" / "explore the build dir" are
    # read requests (a build/feature noun, no coordinating action) and must NOT
    # route; "explore and ship", "discover features to build", "explore the repo
    # for improvements" carry a coordinated build action and DO route.
    _has_build_intent=0
    if [ "$_has_explore" -eq 1 ]; then
        for _bi in "and ship" "to ship" "for ship" \
                   "and build" "to build" "for build" \
                   "and implement" "to implement" "for implement" \
                   "and improve" "to improve" "for improvement" "for improvements" \
                   "and enhance" "to enhance" "for enhancement" "for enhancements"; do
            case "$text_lc" in
                *"$_bi"*) _has_build_intent=1; break ;;
            esac
        done
    fi
    if [ "$_has_explore" -eq 1 ] && [ "$_has_build_intent" -eq 1 ]; then
        if is_imperative "$text_lc"; then
            emit_classify_json true autospec-explore explore imperative 0.7 "$is_auto" "" explore-confirm
        else
            emit_classify_json false autospec-explore explore incidental 0.2 false "" ""
        fi
        return 0
    fi

    # ── 'fix' — imperative bug-fix intent → /autospec end-to-end (D1, #954) ──────
    # Whole-word imperative `fix` routes to the `/autospec` umbrella (spec →
    # issues → run): a fix needs scoping before implementation, unlike the bare
    # implement/build/ship verbs which route straight to autospec-run. NO gate,
    # NO confirm (autospec is not a perpetual loop) — the standard one-line
    # opt-out applies. Confidence 0.7.
    #
    # PLACEMENT: evaluated AFTER explore/refine/run (above) so "fix it by
    # exploring..." and combined-refine phrasings keep their more-specific route,
    # and BEFORE the implement/build/ship chain below so an incidental build/ship
    # word in a fix request does not steal it. Mirrors the explore gate's
    # explicit emit-and-return shape.
    #
    # Trivial-fix suppressors (the negative table — false-negative biased): tiny
    # cosmetic fixes are too small to warrant the full pipeline and must stay
    # plain. Implemented branch-local (mirroring the run-suppressed objects gate)
    # so they short-circuit BEFORE the route emits. The inherited D4
    # question/negation/past guards in is_imperative() cover "did you fix it?",
    # "don't fix that yet", and "I fixed it" (past tense — 'fixed' is not the
    # whole word 'fix').
    # Two tiers of suppression — different exit semantics:
    #
    #   (a) Cosmetic table → HARD no-match (stay plain, per spec §D1). These are
    #       complete trivial-fix requests; emit match:false and return.
    #   (b) Descriptive/deferred ("a fix" the noun, "... later" the deferral) →
    #       SKIP the fix route but FALL THROUGH to the downstream
    #       implement/build/ship chain, so a co-occurring imperative verb still
    #       routes ("ship a fix" must keep its ship → autospec-run route — #954
    #       peer-review regression). A bare "this needs a fix" has no other verb
    #       and falls through to the no-verb-matched no-match anyway.
    _fix_skip=0
    if has_word "$text_lc" "fix"; then
        # (a) Trivial-fix cosmetic table (spec §D1 pins the first seven; "fix the
        # whitespace" added during #954 adversarial self-review as a cosmetic
        # sibling of indentation/formatting). False-negative biased; hard exit.
        for _tf in "fix this typo" "fix the typo" "fix the comment" \
                   "fix the indentation" "fix the formatting" "quick fix" \
                   "fix the spelling" "fix the whitespace"; do
            case "$text_lc" in
                *"$_tf"*) emit_classify_json false "" "" none 0 false "" ""; return 0 ;;
            esac
        done
        # (b) Descriptive/deferred 'fix' phrasings surfaced by #954 self-review
        # (the #909 noun-co-occurrence lesson): "a fix" is the NOUN, not the
        # imperative verb; "... later" is a deferral, not a now-action. Skip the
        # fix route and let the downstream chain decide — do NOT hard-exit here,
        # so "ship a fix" keeps its ship route.
        for _fd in "a fix" "later"; do
            case "$text_lc" in
                *"$_fd"*) _fix_skip=1; break ;;
            esac
        done
    fi
    if has_word "$text_lc" "fix" && [ "$_fix_skip" -eq 0 ]; then
        if is_imperative "$text_lc"; then
            emit_classify_json true autospec fix imperative 0.7 "$is_auto" "" ""
        else
            emit_classify_json false autospec fix incidental 0.2 false "" ""
        fi
        return 0
    fi

    # ── GitHub Projects board URL — route to /autospec-project (Task 12) ────
    # A message containing a GitHub Projects v2 board URL
    # (https://github.com/(orgs|users)/<name>/projects/<n>, optional
    # /views/<n> suffix) carries board intent. Checked BEFORE the generic
    # autospec/implement/build/ship branch below so this more specific route
    # wins over a bare "autospec" or "ship" word in the same message (the
    # acceptance phrase "autospec ship this project for me: <url>" contains
    # both). URL + ship verb -> ship mode (autonomous multi-repo drain); URL
    # alone -> bare resolve mode (zero mutation, prints the plan). The
    # asymmetry is deliberate: reading a board is not a commitment to drain
    # it. Extracted from the ORIGINAL (non-lower-cased) argument so org/user
    # casing in the URL is preserved for the caller.
    _project_url="$(printf '%s' "$1" | grep -oE 'https://github\.com/(orgs|users)/[A-Za-z0-9._-]+/projects/[0-9]+(/views/[0-9]+)?' | head -n1 || true)"
    if [ -n "$_project_url" ]; then
        _has_ship_verb=0
        if has_word "$text_lc" "ship" || has_word "$text_lc" "build" \
            || has_word "$text_lc" "implement"; then
            _has_ship_verb=1
        fi
        if [ "$_has_ship_verb" -eq 1 ]; then
            if is_imperative "$text_lc"; then
                emit_classify_json true autospec-project project-ship imperative 0.85 "$is_auto" "" ""
            else
                emit_classify_json false autospec-project project-ship incidental 0.2 false "" ""
            fi
        else
            if is_imperative "$text_lc"; then
                emit_classify_json true autospec-project project-resolve imperative 0.7 false "" ""
            else
                emit_classify_json false autospec-project project-resolve incidental 0.2 false "" ""
            fi
        fi
        return 0
    fi

    # Existing branches (preserved in order).
    if has_word "$text_lc" "autospec"; then
        skill="autospec"; trigger="autospec"
    elif has_word "$text_lc" "implement" || has_word "$text_lc" "build" \
        || has_word "$text_lc" "ship"; then
        skill="autospec-run"
        if has_word "$text_lc" "implement"; then trigger="implement"
        elif has_word "$text_lc" "build"; then trigger="build"
        else trigger="ship"; fi
    # ── 4. Refine words ─────────────────────────────────────────────────────
    # After implement/build/ship so those more-specific verbs take precedence;
    # before design/spec so refine doesn't shadow them.
    elif has_word "$text_lc" "refine" || has_word "$text_lc" "optimize" \
        || has_word "$text_lc" "polish" || has_word "$text_lc" "improve" \
        || has_word "$text_lc" "tune"; then
        skill="autospec-refine"
        if has_word "$text_lc" "refine"; then trigger="refine"
        elif has_word "$text_lc" "optimize"; then trigger="optimize"
        elif has_word "$text_lc" "polish"; then trigger="polish"
        elif has_word "$text_lc" "improve"; then trigger="improve"
        else trigger="tune"; fi
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

    # No verb matched — but if autonomous-phrasing is present AND the request
    # is imperative, default-route to the /autospec umbrella with autonomous=true
    # so the operator gets the end-to-end pipeline.
    if [ -z "$skill" ]; then
        if [ "$is_auto" = "true" ] && is_imperative "$text_lc"; then
            emit_classify_json true autospec autospec imperative 0.6 true "" ""
        else
            emit_classify_json false "" "" none 0 false "" ""
        fi
        return 0
    fi

    # A verb matched — apply the intent gate.
    if is_imperative "$text_lc"; then
        emit_classify_json true "$skill" "$trigger" imperative 0.8 "$is_auto" "$chain" "$gate"
    else
        emit_classify_json false "$skill" "$trigger" incidental 0.2 false "$chain" "$gate"
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
