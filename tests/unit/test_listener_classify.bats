#!/usr/bin/env bats
# tests/unit/test_listener_classify.bats — classify-mode tests for
# scripts/listener-match.sh --classify, covering the new verbs added in
# issue #852: refine/optimize/polish/improve/tune → autospec-refine,
# run (scoped and bare) → autospec-run, and combined refine+run → chain.
#
# Style follows tests/unit/test_listener_keywords.bats:
#   setup() resolves REPO_ROOT + MATCH
#   run "$MATCH" --classify "<phrase>"
#   assert with jq

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    MATCH="$REPO_ROOT/scripts/listener-match.sh"
}

# ── Positive: refine verbs → autospec-refine ─────────────────────────────────

@test "classify: 'optimize the fleet config' → autospec-refine" {
    run "$MATCH" --classify "optimize the fleet config"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'refine the prompt' → autospec-refine" {
    run "$MATCH" --classify "refine the prompt"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'polish it up' → autospec-refine" {
    run "$MATCH" --classify "polish it up"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'tune the prompt' → autospec-refine" {
    run "$MATCH" --classify "tune the prompt"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'improve the launcher' → autospec-refine" {
    run "$MATCH" --classify "improve the launcher"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

# ── Positive: run (scoped phrases) → autospec-run, gate=null ─────────────────

@test "classify: 'run it' → autospec-run, gate null" {
    run "$MATCH" --classify "run it"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

@test "classify: 'run autospec' → autospec-run, gate null" {
    run "$MATCH" --classify "run autospec"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

@test "classify: 'run the loop' → autospec-run, gate null" {
    run "$MATCH" --classify "run the loop"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

@test "classify: 'drain the queue' → autospec-run, gate null" {
    run "$MATCH" --classify "drain the queue"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

# ── Positive: run (bare, context-gated) → autospec-run, gate=auto-implement-open

@test "classify: 'run' → autospec-run, gate=auto-implement-open" {
    run "$MATCH" --classify "run"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "auto-implement-open" ]
}

@test "classify: 'please run' → autospec-run, gate=auto-implement-open" {
    run "$MATCH" --classify "please run"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "auto-implement-open" ]
}

# ── Suppressed run-objects: must NOT be autospec-run ─────────────────────────

@test "classify: 'run the tests' → clean no-match (match:false, skill:null)" {
    run "$MATCH" --classify "run the tests"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "null" ]
}

@test "classify: 'run the build' → clean no-match (embedded 'build' must not leak autospec-run)" {
    run "$MATCH" --classify "run the build"
    [ "$status" -eq 0 ]
    # Regression: a suppressed "run the <object>" must be a clean no-match —
    # the embedded 'build' verb must NOT re-trigger autospec-run as stale
    # metadata. match:false AND skill:null.
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "null" ]
}

@test "classify: 'run lint' → NOT autospec-run" {
    run "$MATCH" --classify "run lint"
    [ "$status" -eq 0 ]
    skill="$(printf '%s' "$output" | jq -r .skill)"
    [ "$skill" != "autospec-run" ]
}

@test "classify: 'run the server' → NOT autospec-run" {
    run "$MATCH" --classify "run the server"
    [ "$status" -eq 0 ]
    skill="$(printf '%s' "$output" | jq -r .skill)"
    [ "$skill" != "autospec-run" ]
}

@test "classify: 'run the dev server' → NOT autospec-run" {
    run "$MATCH" --classify "run the dev server"
    [ "$status" -eq 0 ]
    skill="$(printf '%s' "$output" | jq -r .skill)"
    [ "$skill" != "autospec-run" ]
}

@test "classify: 'run the migrations' → NOT autospec-run" {
    run "$MATCH" --classify "run the migrations"
    [ "$status" -eq 0 ]
    skill="$(printf '%s' "$output" | jq -r .skill)"
    [ "$skill" != "autospec-run" ]
}

# ── Combined refine+run → skill=autospec-refine, chain=autospec-run ──────────

@test "classify: 'refine and run the queue' → autospec-refine, chain=autospec-run" {
    run "$MATCH" --classify "refine and run the queue"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .chain)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'tune it up' → autospec-refine, chain=autospec-run" {
    run "$MATCH" --classify "tune it up"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .chain)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

@test "classify: 'optimize and run' → autospec-refine, chain=autospec-run" {
    run "$MATCH" --classify "optimize and run"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-refine" ]
    [ "$(printf '%s' "$output" | jq -r .chain)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
}

# ── Negatives via intent gate ─────────────────────────────────────────────────

@test "classify: 'I optimized it already' (past tense) → match:false" {
    run "$MATCH" --classify "I optimized it already"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'should we refine?' (question) → match:false" {
    run "$MATCH" --classify "should we refine?"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"don't run it\" (negation) → match:false" {
    run "$MATCH" --classify "don't run it"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'the optimization is done' (descriptive/past) → match:false" {
    run "$MATCH" --classify "the optimization is done"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# ── New JSON fields present (chain / gate always emitted) ─────────────────────

@test "classify: existing verb 'implement X' → chain and gate fields present" {
    run "$MATCH" --classify "implement the auth module"
    [ "$status" -eq 0 ]
    # Both new fields must be present (null or string).
    printf '%s' "$output" | jq -e 'has("chain")' >/dev/null
    printf '%s' "$output" | jq -e 'has("gate")' >/dev/null
}

@test "classify: no-match phrase → chain and gate fields present and null" {
    run "$MATCH" --classify "hello world"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .chain)" = "null" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

# ── Refine verb: chain=null when no combined phrase ───────────────────────────

@test "classify: 'refine the prompt' → chain=null (no combined trigger)" {
    run "$MATCH" --classify "refine the prompt"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .chain)" = "null" ]
}

# ── Explore/discover intent → autospec-explore, gate=explore-confirm (#909) ──
#
# CLAIM table: explore/discover co-occurring with build/ship/feature intent
# routes to autospec-explore carrying the explore-confirm gate. The gate value
# is consumed byte-for-byte by the autospec-listen trio (#910), so trigger,
# intent, and gate are all asserted.

@test "classify: 'explore and ship new features' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "explore and ship new features"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "explore" ]
    [ "$(printf '%s' "$output" | jq -r .intent)" = "imperative" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

@test "classify: 'discover features to build' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "discover features to build"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "explore" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

@test "classify: 'autonomously explore the repo for improvements' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "autonomously explore the repo for improvements"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

@test "classify: 'go explore and build improvements' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "go explore and build improvements"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

@test "classify: 'discover and implement enhancements' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "discover and implement enhancements"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "explore" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

@test "classify: 'start exploring features to ship' → autospec-explore, gate=explore-confirm" {
    run "$MATCH" --classify "start exploring features to ship"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-explore" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "explore-confirm" ]
}

# ── Explore/discover SUPPRESS table — THE CRITICAL SAFETY GUARD (#909) ───────
#
# Every read/understand/conversational/interrogative/negated/past-tense explore
# phrasing MUST yield match:false (no route). A false positive here would
# auto-launch a perpetual PR-shipping loop on an isolated sandbox branch. Do
# NOT weaken these assertions.

@test "classify: 'explore the codebase' → match:false (read intent, no route)" {
    run "$MATCH" --classify "explore the codebase"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'explore this file' → match:false (read intent, no route)" {
    run "$MATCH" --classify "explore this file"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'explore the data' → match:false (read intent, no route)" {
    run "$MATCH" --classify "explore the data"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'explore options' → match:false (no build intent, no route)" {
    run "$MATCH" --classify "explore options"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'let me explore' → match:false (no build intent, no route)" {
    run "$MATCH" --classify "let me explore"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"I'll explore later\" → match:false (no build intent, no route)" {
    run "$MATCH" --classify "I'll explore later"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'exploratory test' → match:false (not the explore verb, no route)" {
    run "$MATCH" --classify "exploratory test"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"let's do exploratory testing\" → match:false (no route)" {
    run "$MATCH" --classify "let's do exploratory testing"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'the exploration is done' → match:false (past/descriptive)" {
    run "$MATCH" --classify "the exploration is done"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'should we explore?' → match:false (interrogative)" {
    run "$MATCH" --classify "should we explore?"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"don't explore yet\" → match:false (negated)" {
    run "$MATCH" --classify "don't explore yet"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'should we explore and ship?' → match:false (interrogative beats build intent)" {
    run "$MATCH" --classify "should we explore and ship?"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"don't explore and build yet\" → match:false (negation beats build intent)" {
    run "$MATCH" --classify "don't explore and build yet"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# Incidental build/feature NOUN after explore (no coordinating action) must NOT
# route — these slipped past a naive co-occurrence gate and are the real-world
# misfire risk the connector requirement closes.

@test "classify: 'explore the feature flags' → match:false (feature is an incidental noun)" {
    run "$MATCH" --classify "explore the feature flags"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'explore the build directory' → match:false (build is an incidental noun)" {
    run "$MATCH" --classify "explore the build directory"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# "explore the test build output" contains the bare verb 'build', which the
# PRE-EXISTING build→autospec-run branch claims (out of #909's scope). The
# safety property #909 owns is narrower and absolute: it must NOT route to
# autospec-explore (no perpetual ship-loop launch).
@test "classify: 'explore the test build output' → NOT autospec-explore" {
    run "$MATCH" --classify "explore the test build output"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" != "autospec-explore" ]
}

@test "classify: \"let's explore how the build works\" → match:false (read intent)" {
    run "$MATCH" --classify "let's explore how the build works"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# ── 'fix' intent → /autospec end-to-end, NO gate (#954, spec §D1) ────────────
#
# CLAIM table: imperative whole-word `fix` routes to the `/autospec` umbrella
# (spec → issues → run) — a fix needs scoping before implementation. The route
# carries trigger=fix, intent=imperative, confidence=0.7, and NO gate / NO
# confirm (autospec is not a perpetual loop). Evaluated AFTER explore/refine/run
# so it never cannibalizes the more-specific branches.

@test "classify: 'fix the login flow' → autospec, intent=imperative, gate=null" {
    run "$MATCH" --classify "fix the login flow"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "fix" ]
    [ "$(printf '%s' "$output" | jq -r .intent)" = "imperative" ]
    [ "$(printf '%s' "$output" | jq -r .confidence)" = "0.7" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

@test "classify: 'fix the race condition in the watchdog' → autospec, gate=null" {
    run "$MATCH" --classify "fix the race condition in the watchdog"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "fix" ]
    [ "$(printf '%s' "$output" | jq -r .intent)" = "imperative" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

@test "classify: 'please fix the broken pagination' → autospec, gate=null" {
    run "$MATCH" --classify "please fix the broken pagination"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "fix" ]
    [ "$(printf '%s' "$output" | jq -r .intent)" = "imperative" ]
    [ "$(printf '%s' "$output" | jq -r .gate)" = "null" ]
}

# ── 'fix' SUPPRESS table — THE PERMANENT SAFETY GUARD (#954, spec §D1) ────────
#
# Trivial-fix phrasings (typo/comment/indentation/formatting/spelling/quick fix)
# are too small to warrant the /autospec spec→issues→run pipeline and MUST yield
# match:false (stay plain). Plus the inherited D4 question/negation/past guards
# ("did you fix it?", "don't fix that yet", "I fixed it"). Do NOT weaken these —
# a false positive here launches a full end-to-end pipeline on chatter.

@test "classify: 'fix this typo' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix this typo"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'fix the typo' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix the typo"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'fix the comment' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix the comment"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'fix the indentation' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix the indentation"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'fix the formatting' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix the formatting"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'quick fix' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "quick fix"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'fix the spelling' → match:false (trivial-fix suppressor)" {
    run "$MATCH" --classify "fix the spelling"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# Inherited D4 guards applied to 'fix'.

@test "classify: 'did you fix it?' → match:false (interrogative)" {
    run "$MATCH" --classify "did you fix it?"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# No-punctuation interrogative regression (#954 peer-review): "did you fix it"
# without a trailing '?' must still be caught by the interrogative lead-in.
@test "classify: 'did you fix it' (no ?) → match:false (interrogative lead-in)" {
    run "$MATCH" --classify "did you fix it"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"don't fix that yet\" → match:false (negated)" {
    run "$MATCH" --classify "don't fix that yet"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'I fixed it' → match:false (past tense — not the fix verb)" {
    run "$MATCH" --classify "I fixed it"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# Additional suppressors surfaced by #954 adversarial self-review (the #909
# noun-co-occurrence lesson). "fix the whitespace" is a cosmetic sibling of
# indentation/formatting; "a fix" is the noun not the imperative verb;
# "i'll fix … later" / "fix … later" is a deferral, not a now-action.

@test "classify: 'fix the whitespace' → match:false (cosmetic suppressor)" {
    run "$MATCH" --classify "fix the whitespace"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'this needs a fix' → match:false ('a fix' is a noun, not imperative)" {
    run "$MATCH" --classify "this needs a fix"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: \"I'll fix it later\" → match:false (deferred, not a now-action)" {
    run "$MATCH" --classify "I'll fix it later"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

@test "classify: 'can you fix the login bug' → match:false (interrogative)" {
    run "$MATCH" --classify "can you fix the login bug"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# Positive guard: a genuine imperative fix with surrounding words still routes.
@test "classify: \"let's fix the deploy pipeline\" → autospec (genuine imperative)" {
    run "$MATCH" --classify "let's fix the deploy pipeline"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "fix" ]
}

# Regression guard (#954 peer-review): the descriptive "a fix" suppressor must
# NOT swallow a co-occurring build/ship/implement verb. "ship a fix" keeps its
# pre-existing ship → autospec-run route (the fix branch falls through, it does
# not hard no-match).
@test "classify: 'ship a fix' → autospec-run (ship route preserved, not swallowed)" {
    run "$MATCH" --classify "ship a fix"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
}

# ── GitHub Projects board URL routing (Task 12) ──────────────────────────────
# A message with a GitHub Projects v2 board URL
# (https://github.com/(orgs|users)/<name>/projects/<n>, optional
# /views/<n>) routes to autospec-project. URL + ship/build/implement verb ->
# trigger project-ship; URL alone -> trigger project-resolve, NEVER ship.
# Checked before the generic autospec/implement/build/ship branch so it wins
# over a bare "autospec" or "ship" word in the same message.

@test "classify: operator's literal acceptance phrase → autospec-project, project-ship" {
    run "$MATCH" --classify "autospec ship this project for me: https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

@test "classify: 'ship <url>' → autospec-project, project-ship" {
    run "$MATCH" --classify "ship https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

@test "classify: 'implement <url>' → autospec-project, project-ship" {
    run "$MATCH" --classify "implement https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

@test "classify: 'build everything on <url>' → autospec-project, project-ship" {
    run "$MATCH" --classify "build everything on https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

@test "classify: 'ship' + a /users/ board URL → autospec-project, project-ship" {
    run "$MATCH" --classify "ship https://github.com/users/berlinguyinca/projects/7"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

@test "classify: 'ship' + a board URL with a /views/3 suffix → autospec-project, project-ship" {
    run "$MATCH" --classify "ship https://github.com/orgs/InferWeave/projects/2/views/3"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-ship" ]
}

# Asymmetry (the safety property): a board URL with NO ship/build/implement
# verb resolves and prints the plan — it must NEVER route to ship.

@test "classify: bare board URL alone → autospec-project, project-resolve (never ship)" {
    run "$MATCH" --classify "https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-resolve" ]
}

@test "classify: \"what's on <url>\" → autospec-project, project-resolve (never ship)" {
    run "$MATCH" --classify "what's on https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-resolve" ]
}

@test "classify: 'show me <url>' → autospec-project, project-resolve (never ship)" {
    run "$MATCH" --classify "show me https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-resolve" ]
}

@test "classify: 'look at <url>' → autospec-project, project-resolve (never ship)" {
    run "$MATCH" --classify "look at https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-project" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "project-resolve" ]
}

# Negative: non-Projects GitHub URLs must NOT route to autospec-project at all
# — only /projects/<n> board URLs qualify, not issues, PRs, plain repos, or a
# bare /repositories listing URL.

@test "classify: 'ship' + a GitHub issue URL → NOT autospec-project" {
    run "$MATCH" --classify "ship https://github.com/berlinguyinca/autospec/issues/42"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" != "autospec-project" ]
}

@test "classify: 'ship' + a GitHub PR URL → NOT autospec-project" {
    run "$MATCH" --classify "ship https://github.com/berlinguyinca/autospec/pull/42"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" != "autospec-project" ]
}

@test "classify: 'ship' + a plain repo URL → NOT autospec-project" {
    run "$MATCH" --classify "ship https://github.com/berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" != "autospec-project" ]
}

@test "classify: 'ship' + a /repositories listing URL → NOT autospec-project" {
    run "$MATCH" --classify "ship https://github.com/orgs/InferWeave/repositories"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .skill)" != "autospec-project" ]
}

# Negation must still win over the ship+URL co-occurrence: "please don't ship
# yet, just look at <url>" must produce no route at all.

@test "classify: \"please don't ship yet, just look at <url>\" → match:false, no route" {
    run "$MATCH" --classify "please don't ship yet, just look at https://github.com/orgs/InferWeave/projects/2"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "false" ]
}

# Regression guards: a ship verb or bare "autospec" phrase with NO board URL
# must keep its pre-existing route unchanged — this branch must never hijack
# them.

@test "classify: 'ship this feature for me' (no URL) → autospec-run, ship (unchanged)" {
    run "$MATCH" --classify "ship this feature for me"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec-run" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "ship" ]
}

@test "classify: 'autospec do the release' (no URL) → autospec, autospec (unchanged)" {
    run "$MATCH" --classify "autospec do the release"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r .match)" = "true" ]
    [ "$(printf '%s' "$output" | jq -r .skill)" = "autospec" ]
    [ "$(printf '%s' "$output" | jq -r .trigger)" = "autospec" ]
}
