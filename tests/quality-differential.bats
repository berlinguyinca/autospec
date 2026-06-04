#!/usr/bin/env bats
# tests/quality-differential.bats — coverage for scripts/quality-differential.sh
# (spec §5 D4: the boilerplate guard).
#
# Real files, no mocks. Two layers:
#   * Self-test with the synthetic step — a negative-path pair: the
#     signal-bearing fixture PASSES, the canned-string fixture FAILS, and the
#     harness exits non-zero because ANY fixture failing is a fail.
#   * The real first consumer, refine-lenses — per spec §7 the deterministic
#     lens path is EXPECTED to fail (it appends canned boilerplate), so the
#     harness exiting NON-ZERO on refine-lenses is the documented success
#     condition for this issue.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/quality-differential.sh"
SYNTH_FIX="$REPO_ROOT/tests/fixtures/quality-diff/synth"
REFINE_FIX="$REPO_ROOT/tests/fixtures/quality-diff/refine-lenses"

setup() {
    TMPWORK="$(mktemp -d -t qd-bats.XXXXXX)"
}

teardown() {
    if [ -n "${TMPWORK:-}" ]; then
        rm -rf "$TMPWORK"
    fi
}

# ── usage / arg handling ──────────────────────────────────────────

@test "missing --step errors with usage exit 2" {
    run bash "$BIN" --fixtures "$SYNTH_FIX"
    [ "$status" -eq 2 ]
}

@test "missing --fixtures errors with usage exit 2" {
    run bash "$BIN" --step synthetic
    [ "$status" -eq 2 ]
}

@test "unknown step errors exit 2" {
    run bash "$BIN" --step nope --fixtures "$SYNTH_FIX"
    [ "$status" -eq 2 ]
}

@test "nonexistent fixtures dir errors exit 2" {
    run bash "$BIN" --step synthetic --fixtures "$TMPWORK/does-not-exist"
    [ "$status" -eq 2 ]
}

@test "fixtures dir with no usable fixtures errors exit 2" {
    mkdir -p "$TMPWORK/empty"
    run bash "$BIN" --step synthetic --fixtures "$TMPWORK/empty"
    [ "$status" -eq 2 ]
}

@test "--help exits 0 and prints usage" {
    run bash "$BIN" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "step must keep/restore its LLM path"
}

# ── synthetic self-test: negative-path pair ───────────────────────

@test "synthetic step over full synth dir exits non-zero (any fail => fail)" {
    run bash "$BIN" --step synthetic --fixtures "$SYNTH_FIX"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "FAIL .*fail-canned"
    echo "$output" | grep -q "PASS .*pass-signal"
}

@test "synthetic: signal-bearing fixture alone PASSES (exit 0)" {
    mkdir -p "$TMPWORK/only-pass/pass-signal"
    cp "$SYNTH_FIX/pass-signal/input"      "$TMPWORK/only-pass/pass-signal/input"
    cp "$SYNTH_FIX/pass-signal/llm-golden" "$TMPWORK/only-pass/pass-signal/llm-golden"
    cp "$SYNTH_FIX/pass-signal/assert.sh"  "$TMPWORK/only-pass/pass-signal/assert.sh"
    run bash "$BIN" --step synthetic --fixtures "$TMPWORK/only-pass"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "deterministic path OK"
}

@test "synthetic: canned-string fixture alone FAILS (exit 1)" {
    mkdir -p "$TMPWORK/only-fail/fail-canned"
    cp "$SYNTH_FIX/fail-canned/input"      "$TMPWORK/only-fail/fail-canned/input"
    cp "$SYNTH_FIX/fail-canned/llm-golden" "$TMPWORK/only-fail/fail-canned/llm-golden"
    cp "$SYNTH_FIX/fail-canned/assert.sh"  "$TMPWORK/only-fail/fail-canned/assert.sh"
    run bash "$BIN" --step synthetic --fixtures "$TMPWORK/only-fail"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "canned boilerplate marker present"
}

# ── assert.sh is signal-property, NOT string equality ─────────────

@test "assert.sh passes when det-output differs from golden but shares >=2 content tokens" {
    # det output is paraphrased (no string equality with golden) yet on-topic.
    printf 'A read-through cache fronts the profile API to cut database load.\n' > "$TMPWORK/det"
    printf 'Implement a read-through cache for the user profile API endpoint.\n' > "$TMPWORK/golden"
    run bash "$SYNTH_FIX/pass-signal/assert.sh" "$TMPWORK/det" "$TMPWORK/golden"
    [ "$status" -eq 0 ]
    # And prove it is NOT string equality: the two files differ.
    run diff "$TMPWORK/det" "$TMPWORK/golden"
    [ "$status" -ne 0 ]
}

@test "assert.sh fails on generic output lacking content-token overlap" {
    printf 'Please refine the prompt below per the rubric.\n' > "$TMPWORK/det"
    printf 'Implement a streaming CSV export for the inventory dashboard.\n' > "$TMPWORK/golden"
    run bash "$SYNTH_FIX/pass-signal/assert.sh" "$TMPWORK/det" "$TMPWORK/golden"
    [ "$status" -ne 0 ]
}

@test "assert.sh fails on any canned boilerplate marker regardless of token overlap" {
    printf 'Cache the profile API endpoint.\nWhat happens on empty input? Add a test.\n' > "$TMPWORK/det"
    printf 'Cache the profile API endpoint with a TTL.\n' > "$TMPWORK/golden"
    run bash "$SYNTH_FIX/pass-signal/assert.sh" "$TMPWORK/det" "$TMPWORK/golden"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "canned boilerplate"
}

# ── refine-lenses real consumer: documented FAILING verdict ───────

@test "refine-lenses fixtures number >= 3" {
    count="$(ls -d "$REFINE_FIX"/*/ 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" -ge 3 ]
}

@test "every refine-lenses fixture ships input + assert.sh" {
    for d in "$REFINE_FIX"/*/; do
        [ -f "${d}input" ]
        [ -f "${d}assert.sh" ]
    done
}

@test "refine-lenses step exits NON-ZERO (spec §7 documented verdict: keep LLM path)" {
    run bash "$BIN" --step refine-lenses --fixtures "$REFINE_FIX"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "keep/restore LLM path"
    # The deterministic refine lens path appends canned boilerplate -> the
    # boilerplate marker is the proof of why it fails.
    echo "$output" | grep -q "canned boilerplate marker present"
}
