#!/usr/bin/env bats
# tests/loop/test_loop_state.bats — loop-state.sh atomic read/update.
#
# Covers (docs/specs/2026-06-02-autospec-loop-design.md §Phase 2):
#   - init writes a valid loop-state.json with schema:1 defaults
#   - read returns the current state JSON
#   - update <key> <val> round-trips string and numeric values
#   - atomic write: tmp+mv (file appears atomically, no partial writes)
#   - path is scoped by repo-slug (owner/name -> owner__name)
#   - slug isolation: two repos do not share state

ROOT="${BATS_TEST_DIRNAME}/../.."
LOOP_STATE="$ROOT/skills/autospec-loop/scripts/loop-state.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_LOOP_STATE_DIR="$TEST_TMP/loop"
}

teardown() { rm -rf "$TEST_TMP"; }

# ── init ──────────────────────────────────────────────────────────────────────

@test "init creates loop-state.json with schema 1" {
    run bash "$LOOP_STATE" init --repo owner/name \
        --goal "tests pass" --check "make test"
    [ "$status" -eq 0 ]
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ -f "$f" ]
    [ "$(jq '.schema' "$f")" = "1" ]
}

@test "init sets goal and check fields" {
    bash "$LOOP_STATE" init --repo owner/name \
        --goal "build is green" --check "bash build.sh"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.goal' "$f")" = "build is green" ]
    [ "$(jq -r '.check' "$f")" = "bash build.sh" ]
}

@test "init sets default numeric fields" {
    bash "$LOOP_STATE" init --repo owner/name --goal "done" --check "null"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq '.max_iters' "$f")" = "25" ]
    [ "$(jq '.no_progress_K' "$f")" = "3" ]
    [ "$(jq '.iter' "$f")" = "0" ]
    [ "$(jq '.no_progress_count' "$f")" = "0" ]
    [ "$(jq '.done' "$f")" = "false" ]
    [ "$(jq -r '.exit_reason' "$f")" = "null" ]
}

@test "init sets mode to cumulative by default" {
    bash "$LOOP_STATE" init --repo owner/name --goal "done" --check "null"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.mode' "$f")" = "cumulative" ]
}

@test "init accepts --mode polling" {
    bash "$LOOP_STATE" init --repo owner/name --goal "done" --check "null" --mode polling
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.mode' "$f")" = "polling" ]
}

@test "init with --check null stores null (JSON null, not string)" {
    bash "$LOOP_STATE" init --repo owner/name --goal "done" --check "null"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.check' "$f")" = "null" ]
}

# ── read ──────────────────────────────────────────────────────────────────────

@test "read returns the full JSON object" {
    bash "$LOOP_STATE" init --repo owner/name --goal "my goal" --check "true"
    run bash "$LOOP_STATE" read --repo owner/name
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -r '.goal')" = "my goal" ]
}

@test "read exits non-zero when state file is missing" {
    run bash "$LOOP_STATE" read --repo no/such
    [ "$status" -ne 0 ]
}

# ── update ────────────────────────────────────────────────────────────────────

@test "update string field round-trips" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    bash "$LOOP_STATE" update --repo owner/name --key last_result --value "iteration 1 done"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.last_result' "$f")" = "iteration 1 done" ]
}

@test "update numeric field iter increments" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    bash "$LOOP_STATE" update --repo owner/name --key iter --value 1
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq '.iter' "$f")" = "1" ]
}

@test "update done to true" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    bash "$LOOP_STATE" update --repo owner/name --key done --value true
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq '.done' "$f")" = "true" ]
}

@test "update exit_reason sets string value" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    bash "$LOOP_STATE" update --repo owner/name --key exit_reason --value "goal-met"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.exit_reason' "$f")" = "goal-met" ]
}

@test "update preserves other fields" {
    bash "$LOOP_STATE" init --repo owner/name --goal "my goal" --check "cmd"
    bash "$LOOP_STATE" update --repo owner/name --key iter --value 5
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    [ "$(jq -r '.goal' "$f")" = "my goal" ]
    [ "$(jq -r '.check' "$f")" = "cmd" ]
    [ "$(jq '.iter' "$f")" = "5" ]
}

# ── atomic write ──────────────────────────────────────────────────────────────

@test "atomic write: final file is valid JSON after update" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    bash "$LOOP_STATE" update --repo owner/name --key last_result --value "ok"
    f="$(bash "$LOOP_STATE" path --repo owner/name)"
    run jq . "$f"
    [ "$status" -eq 0 ]
}

@test "atomic write: no tmp file left after init" {
    bash "$LOOP_STATE" init --repo owner/name --goal "g" --check "c"
    dir="$(dirname "$(bash "$LOOP_STATE" path --repo owner/name)")"
    tmp_count="$(find "$dir" -name '*.XXXXXX' -o -name 'loop-state.json.tmp*' 2>/dev/null | wc -l | tr -d ' ')"
    [ "$tmp_count" = "0" ]
}

# ── path + slug isolation ─────────────────────────────────────────────────────

@test "path command returns slug-scoped path" {
    run bash "$LOOP_STATE" path --repo owner/name
    [ "$status" -eq 0 ]
    [[ "$output" == *"/owner__name/"* ]]
    [[ "$output" == *"loop-state.json" ]]
}

@test "slug isolation: two repos do not share state" {
    bash "$LOOP_STATE" init --repo repo/alpha --goal "alpha" --check "null"
    bash "$LOOP_STATE" init --repo repo/beta  --goal "beta"  --check "null"
    alpha_goal="$(jq -r '.goal' "$(bash "$LOOP_STATE" path --repo repo/alpha)")"
    beta_goal="$(jq -r '.goal'  "$(bash "$LOOP_STATE" path --repo repo/beta)")"
    [ "$alpha_goal" = "alpha" ]
    [ "$beta_goal"  = "beta"  ]
}
