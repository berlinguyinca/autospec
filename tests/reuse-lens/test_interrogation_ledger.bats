#!/usr/bin/env bats
# tests/reuse-lens/test_interrogation_ledger.bats
# TDD coverage for scripts/interrogation-ledger.sh (issue #1442).
#
# Covers:
#   - flag-OFF inertness (AUTOSPEC_REUSE_LENS unset → exit 0, no side effects)
#   - record → report round-trip (verdicts land in ledger, report reads them)
#   - synthetic always-wrong trigger auto-demotes after AUTOSPEC_REUSE_DEMOTE_AFTER
#   - precision subcommand exits non-zero when any trigger should be demoted
#   - ledger write failure warns and exits 0 (best-effort)
#   - validate.sh enumerates tests/reuse-lens/ (gate presence)
#   - wire-in: skills/autospec-run/SKILL.md references interrogation-ledger.sh
#
# Bash rules: set -eu; if/then/fi; no RETURN traps; real temp files (not
# process substitution in [ -f ] — macOS bash 3.2 compat).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/interrogation-ledger.sh"

    # Isolated temp dir per test.
    TEST_TMP="$(mktemp -d)"

    # Point ledger to isolated path via env override.
    LEDGER="$TEST_TMP/interrogation-ledger.jsonl"
    export AUTOSPEC_INTERROGATION_LEDGER="$LEDGER"

    # Arm the feature for tests that need it.
    export AUTOSPEC_REUSE_LENS=1
    export AUTOSPEC_REUSE_PRECISION_FLOOR=0.6
    export AUTOSPEC_REUSE_DEMOTE_AFTER=10

    export TEST_TMP LEDGER SCRIPT REPO_ROOT
}

teardown() {
    rm -rf "$TEST_TMP"
}

# Helper: run the ledger script with all env already exported.
run_ledger() {
    run bash "$SCRIPT" "$@"
}

# ── flag-OFF inertness ────────────────────────────────────────────────────────

@test "flag OFF: AUTOSPEC_REUSE_LENS unset → record exits 0, no ledger created" {
    unset AUTOSPEC_REUSE_LENS
    run_ledger record \
        --issue 99 --pr 200 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld true
    [ "$status" -eq 0 ]
    # Ledger file must NOT be created.
    [ ! -f "$LEDGER" ]
}

@test "flag OFF: AUTOSPEC_REUSE_LENS unset → report exits 0, no output" {
    unset AUTOSPEC_REUSE_LENS
    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "flag OFF: AUTOSPEC_REUSE_LENS unset → precision exits 0, no output" {
    unset AUTOSPEC_REUSE_LENS
    run_ledger precision --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "flag OFF: AUTOSPEC_REUSE_LENS empty string → inert" {
    export AUTOSPEC_REUSE_LENS=""
    run_ledger record \
        --issue 99 --pr 200 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld true
    [ "$status" -eq 0 ]
    [ ! -f "$LEDGER" ]
}

# ── record subcommand ─────────────────────────────────────────────────────────

@test "record: creates ledger file and appends valid compact JSONL" {
    run_ledger record \
        --issue 100 --pr 200 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld true
    [ "$status" -eq 0 ]
    # File must exist (real temp file, not process sub — macOS bash 3.2 safe).
    [ -f "$LEDGER" ]
    local line
    line="$(cat "$LEDGER")"
    # Must be valid JSON.
    printf '%s' "$line" | jq -e . >/dev/null
    # Fields must match.
    printf '%s' "$line" | jq -e '.trigger == "REINVENT_REPO_UTIL"' >/dev/null
    printf '%s' "$line" | jq -e '.verdict == "BLOCK"' >/dev/null
    printf '%s' "$line" | jq -e '.upheld == true' >/dev/null
    printf '%s' "$line" | jq -e '.issue == "100"' >/dev/null
    printf '%s' "$line" | jq -e '.pr == "200"' >/dev/null
    printf '%s' "$line" | jq -e '.ts | type == "number"' >/dev/null
}

@test "record: each call appends exactly one JSONL line (compact single line)" {
    run_ledger record --issue 1 --pr 1 --trigger T1 --verdict BLOCK --upheld true
    run_ledger record --issue 2 --pr 2 --trigger T2 --verdict ADVISE --upheld false
    run_ledger record --issue 3 --pr 3 --trigger T3 --verdict PASS
    [ "$(wc -l < "$LEDGER")" -eq 3 ]
}

@test "record: upheld null (omitted) records null in JSON" {
    run_ledger record --issue 1 --pr 1 --trigger T --verdict BLOCK
    [ "$status" -eq 0 ]
    local line
    line="$(cat "$LEDGER")"
    printf '%s' "$line" | jq -e '.upheld == null' >/dev/null
}

@test "record: write failure warns and exits 0 (best-effort; never blocks)" {
    # Point to a path whose parent directory cannot be created (file in place of dir).
    local bad_path="$TEST_TMP/not-a-dir"
    touch "$bad_path"
    export AUTOSPEC_INTERROGATION_LEDGER="$bad_path/subpath/ledger.jsonl"
    run_ledger record --issue 1 --pr 1 --trigger T --verdict BLOCK --upheld true
    # Must exit 0 even on write failure.
    [ "$status" -eq 0 ]
}

# ── record → report round-trip ────────────────────────────────────────────────

@test "round-trip: record then report shows trigger in table" {
    run_ledger record --issue 10 --pr 20 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld true
    run_ledger record --issue 11 --pr 21 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld true
    run_ledger record --issue 12 --pr 22 --trigger REINVENT_REPO_UTIL --verdict BLOCK --upheld false

    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    # Output must mention the trigger name.
    printf '%s' "$output" | grep -q "REINVENT_REPO_UTIL"
    # Precision = 2/3 ≈ 66% — should show ok (above 0.6 floor, but only 3 < 10 runs).
    printf '%s' "$output" | grep -q "REINVENT_REPO_UTIL.*ok"
}

@test "round-trip: non-BLOCK verdicts are excluded from precision computation" {
    # Only BLOCK verdicts count toward precision; PASS/ADVISE rows are noise.
    run_ledger record --issue 1 --pr 1 --trigger T --verdict PASS
    run_ledger record --issue 2 --pr 2 --trigger T --verdict ADVISE
    run_ledger record --issue 3 --pr 3 --trigger T --verdict BLOCK --upheld true

    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    # T has 1 BLOCK (upheld=true) → precision 100%, total=1 < after=10 → ok.
    printf '%s' "$output" | grep -q "T.*blocks=1.*ok"
}

# ── auto-demotion ─────────────────────────────────────────────────────────────

@test "auto-demote: synthetic always-wrong trigger flags DEMOTE after floor runs" {
    # 10 BLOCK runs, 0 upheld → precision 0% < 0.6 floor, total=10 >= after=10.
    for i in $(seq 1 10); do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger ALWAYS_WRONG --verdict BLOCK --upheld false
    done
    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q "ALWAYS_WRONG.*DEMOTE"
}

@test "auto-demote: trigger with too few runs is NOT demoted (even at 0% precision)" {
    # 9 BLOCK runs, 0 upheld — below the DEMOTE_AFTER=10 threshold.
    for i in $(seq 1 9); do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger ALMOST_WRONG --verdict BLOCK --upheld false
    done
    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q "ALMOST_WRONG.*ok"
}

@test "auto-demote: trigger above precision floor is NOT demoted" {
    # 10 BLOCK runs, 8 upheld → precision 80% > 0.6 floor → no DEMOTE.
    for i in $(seq 1 8); do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger GOOD_TRIGGER --verdict BLOCK --upheld true
    done
    for i in 9 10; do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger GOOD_TRIGGER --verdict BLOCK --upheld false
    done
    run_ledger report --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q "GOOD_TRIGGER.*ok"
}

# ── precision subcommand exit codes ──────────────────────────────────────────

@test "precision: exits 1 when any trigger should be demoted" {
    export AUTOSPEC_REUSE_DEMOTE_AFTER=5
    for i in $(seq 1 5); do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger BAD_TRIGGER --verdict BLOCK --upheld false
    done
    run_ledger precision --ledger "$LEDGER" --floor 0.6 --after 5
    [ "$status" -eq 1 ]
}

@test "precision: exits 0 when no trigger needs demotion" {
    export AUTOSPEC_REUSE_DEMOTE_AFTER=5
    for i in $(seq 1 5); do
        bash "$SCRIPT" record --issue "$i" --pr "$i" \
            --trigger GOOD_TRIGGER --verdict BLOCK --upheld true
    done
    run_ledger precision --ledger "$LEDGER" --floor 0.6 --after 5
    [ "$status" -eq 0 ]
}

@test "precision: empty ledger exits 0 (no triggers, nothing to demote)" {
    run_ledger precision --ledger "$LEDGER"
    [ "$status" -eq 0 ]
}

# ── direct Rust validation owner ─────────────────────────────────────────────

@test "direct Rust validation registers the reuse-lens suite" {
    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    grep -q '"check_reuse_lens_suite"' "$catalog"
    grep -q 'BatsDirectory("tests/reuse-lens")' "$catalog"
}

# ── wire-in: conductor references interrogation-ledger.sh ────────────────────

@test "wire-in: skills/autospec-run/SKILL.md references interrogation-ledger.sh" {
    # Proves the record call site is wired into the fused-review conductor path,
    # not just defined in scripts/ (feedback_feature_wired_to_script_but_never_invoked).
    grep -q 'interrogation-ledger' "$REPO_ROOT/skills/autospec-run/SKILL.md"
}
