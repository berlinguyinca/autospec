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

@test "classify: 'run the tests' → NOT autospec-run" {
    run "$MATCH" --classify "run the tests"
    [ "$status" -eq 0 ]
    skill="$(printf '%s' "$output" | jq -r .skill)"
    [ "$skill" != "autospec-run" ]
}

@test "classify: 'run the build' → not routed to autospec-run (match:false or skill differs)" {
    run "$MATCH" --classify "run the build"
    [ "$status" -eq 0 ]
    # Either no match, or the skill is not autospec-run via run-branch routing.
    match_val="$(printf '%s' "$output" | jq -r .match)"
    skill="$(printf '%s' "$output" | jq -r .skill)"
    # At least one of: match is false, or the skill is something other than
    # autospec-run being routed by the run-scoped/bare branch.
    # We verify the *build* route (not autospec-run via run-path) by checking
    # the trigger is not "run".
    trigger="$(printf '%s' "$output" | jq -r .trigger)"
    [ "$match_val" = "false" ] || [ "$trigger" != "run" ]
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
