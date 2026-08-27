#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-2-5.bats
#
# Regression coverage for the run_metric() path-resolution bug in
# gate-stage-2-5.sh: SCRIPT_DIR/../invariants/$2 pointed at a directory that
# does not exist, so every metric silently took the stub-pass fallback
# ({"passed":true,"skipped":true,"reason":"runner not installed"}) instead of
# ever invoking its real runner. These tests assert the gate now actually
# reaches each real runner (proven by getting a *different* failure mode —
# "refused"/"exited" from the runner itself — never the stub-pass reason)
# and that a v2-enabled bait target makes the overall gate fail.
#
# The runners themselves require a live base_url + {contract,base_url} stdin
# payload that gate-stage-2-5.sh's run_metric() does not construct (a
# separate, pre-existing wiring gap outside this bug's scope), so in this
# environment every metric on every v2-enabled target reports "refused" or
# "exited N" rather than a live pass/fail verdict — see the report for detail.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    GATE="$SCRIPTS_DIR/gate-stage-2-5.sh"
    TARGETS_DIR="$REPO_ROOT/skills/autospec-test/test-targets"
}

# ── Runner paths actually exist where the fixed script looks for them ───────

@test "invariants/run-structural.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/invariants/run-structural.mjs" ]
}

@test "window-contract/run-window.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/window-contract/run-window.mjs" ]
}

@test "crawler-v2/extended-crawler.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/crawler-v2/extended-crawler.mjs" ]
}

@test "contract-symmetry/run-symmetry.mjs exists at the path run_metric now resolves" {
    [ -f "$SCRIPTS_DIR/contract-symmetry/run-symmetry.mjs" ]
}

@test "gate-stage-2-5.sh no longer references the nonexistent ../invariants/\$2 prefix" {
    ! grep -q '\.\./invariants/\$2' "$GATE"
}

# ── target-invariant-bait: metric F path must be actually invoked ───────────

@test "target-invariant-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-invariant-bait: overall gate fails (v2 enabled, no metric silently stub-passes)" {
    run bash "$GATE" "$TARGETS_DIR/target-invariant-bait" < /dev/null
    [ "$status" -eq 1 ]
    printf '%s' "$output" | jq -e '.passed == false' >/dev/null
}

# ── target-window-mismatch-bait ──────────────────────────────────────────────

@test "target-window-mismatch-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-window-mismatch-bait: overall gate fails" {
    run bash "$GATE" "$TARGETS_DIR/target-window-mismatch-bait" < /dev/null
    [ "$status" -eq 1 ]
}

# ── target-greenwash-bait ────────────────────────────────────────────────────

@test "target-greenwash-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-greenwash-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-greenwash-bait: overall gate fails" {
    run bash "$GATE" "$TARGETS_DIR/target-greenwash-bait" < /dev/null
    [ "$status" -eq 1 ]
}

# ── target-contract-symmetry-bait ────────────────────────────────────────────

@test "target-contract-symmetry-bait: gate output is not the stub-pass reason for any metric" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    ! printf '%s' "$output" | grep -q 'runner not installed'
}

@test "target-contract-symmetry-bait: overall gate fails" {
    run bash "$GATE" "$TARGETS_DIR/target-contract-symmetry-bait" < /dev/null
    [ "$status" -eq 1 ]
}

# ── target-clean-pass: no invariants_v2 block, gate must skip (not run at all) ──

@test "target-clean-pass: gate short-circuits as skipped (no invariants_v2 declared)" {
    run bash "$GATE" "$TARGETS_DIR/target-clean-pass" < /dev/null
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.skipped == true and .passed == true' >/dev/null
}

# ── jq "// true" default-operator bug: false is falsy in jq, so `.passed //
# true` silently rewrote every real "passed":false into "true". Assert the
# fixed filter is used instead. ─────────────────────────────────────────────

@test "gate-stage-2-5.sh no longer uses the jq '.passed // true' false-is-falsy bug" {
    ! grep -q '\.passed // true' "$GATE"
}

@test "a metric JSON with explicit passed:false is honored, not coerced to true" {
    result=$(printf '{"metric":"F","passed":false}' | jq -r 'if .passed == null then true else .passed end')
    [ "$result" = "false" ]
}
