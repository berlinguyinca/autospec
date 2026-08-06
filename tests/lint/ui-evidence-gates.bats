#!/usr/bin/env bats
# scripts/ui-evidence-gates.sh — one runner for the five runtime UI gates.
#
# The gates existed for four merges with nothing invoking them: no script, no workflow, only
# five prose steps in the QA cluster that an agent was expected to follow by hand, merging
# five report shapes and five exit conventions itself. This suite pins the runner's contract
# so that stays true no matter which gate is added next.
#
# The gates themselves are covered by their own node:test suites, driving real browsers. What
# is tested here is orchestration: does a missing browser read as unknown rather than pass,
# does one gate's failure survive into the summary, is the status line parseable.

setup() {
    R="$BATS_TEST_DIRNAME/../../scripts/ui-evidence-gates.sh"
    TMP="$(mktemp -d)"
    STUB="$TMP/stubs"
    mkdir -p "$STUB" "$TMP/out"
}

teardown() {
    rm -rf "$TMP"
}

# Write a stub gate that prints $2 and exits $3, so the runner can be driven through every
# outcome without a browser. These are .mjs and are executed by node, because that is how the
# runner invokes a real gate — a bash stub would test a code path that does not exist.
stub() { # stub NAME OUTPUT EXIT
    cat > "$STUB/$1" <<EOF
console.log(process.argv.slice(2).length >= 0 ? "$2" : "");
process.exit($3);
EOF
}

all_stubs() { # all_stubs EXIT — every gate behaves the same way
    for g in ui-motion-evidence.mjs ui-device-evidence.mjs ui-keyboard-evidence.mjs \
             ui-liveregion-evidence.mjs ui-a11y-baseline.mjs; do
        stub "$g" "stub output for $g" "$1"
    done
}

@test "all gates clean: PASS, exit 0" {
    all_stubs 0
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^ui-evidence-gates: PASS'
}

@test "one gate with findings: FAIL, exit 1, and the gate is named" {
    all_stubs 0
    stub ui-keyboard-evidence.mjs "KEYBOARD_TRAP:/runs: cannot tab out" 1
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^ui-evidence-gates: FAIL'
    printf '%s\n' "$output" | grep -q 'keyboard'
}

@test "a missing browser is UNKNOWN, not a pass" {
    # Exit 3 means no evidence was collected. Rolling that into PASS is the exact failure the
    # cluster doc warns about: an absent browser is not a passing grade.
    all_stubs 0
    stub ui-motion-evidence.mjs "Playwright unavailable" 3
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q '^ui-evidence-gates: UNKNOWN'
}

@test "findings outrank unknown: a real failure is not masked by a missing browser" {
    all_stubs 0
    stub ui-motion-evidence.mjs "Playwright unavailable" 3
    stub ui-keyboard-evidence.mjs "KEYBOARD_TRAP:/runs: cannot tab out" 1
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^ui-evidence-gates: FAIL'
}

@test "every gate runs even after one fails" {
    all_stubs 1
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 1 ]
    # A runner that stopped at the first failure would hide the other four, and an operator
    # would fix one thing per run.
    for g in motion device keyboard liveregion a11y; do
        printf '%s\n' "$output" | grep -q "$g"
    done
}

@test "the merged report names every gate, its status and its output" {
    all_stubs 0
    stub ui-device-evidence.mjs "DEVICE_REFLOW:/runs:320px: too wide" 1
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ -f "$TMP/out/ui-evidence-gates.json" ]
    run node -e "
      const r = require('$TMP/out/ui-evidence-gates.json');
      if (r.gates.length !== 5) throw new Error('expected 5 gates, got ' + r.gates.length);
      const d = r.gates.find((g) => g.gate === 'device');
      if (d.status !== 'findings') throw new Error('device status ' + d.status);
      if (!d.output.includes('DEVICE_REFLOW')) throw new Error('device output not captured');
      if (r.status !== 'FAIL') throw new Error('overall ' + r.status);
      console.log('ok');
    "
    [ "$status" -eq 0 ]
}

@test "a gate that exits 0 having verified nothing is UNKNOWN, not ok" {
    # liveregion does exactly this against a server-rendered app: every route is skipped, it
    # exits 0, and "ok liveregion" would report zero coverage as a clean bill of health.
    all_stubs 0
    printf '{"schema":1,"status":"ok","measured":0,"states":[],"findings":[],"skipped":[{"route":"/"}]}\n' \
        > "$TMP/out/liveregion-evidence.json"
    stub ui-liveregion-evidence.mjs "0 of 6 route(s) measured" 0
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q 'verified nothing'
    printf '%s\n' "$output" | grep -q '^ui-evidence-gates: UNKNOWN'
}

@test "a gate binary that is missing is UNKNOWN, not silently skipped" {
    all_stubs 0
    rm "$STUB/ui-a11y-baseline.mjs"
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q 'a11y'
}

@test "--base-url is required" {
    run bash "$R" --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -qi 'base-url'
}

@test "the status line is the last line, so a pipeline can parse it" {
    all_stubs 0
    run bash "$R" --base-url http://x --routes / --gates-dir "$STUB" --report-dir "$TMP/out"
    last="$(printf '%s\n' "$output" | tail -1)"
    printf '%s' "$last" | grep -q '^ui-evidence-gates: PASS'
}
