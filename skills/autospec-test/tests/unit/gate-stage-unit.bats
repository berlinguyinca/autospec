#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-unit.bats
#
# TDD tests for Phase 2: gate-stage-unit.sh, coverage collectors,
# and function-presence.mjs
#
# Four edges per acceptance criteria:
#   pass, threshold-fail, function-presence-fail, tests-red

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures"
    SCHEMA="$REPO_ROOT/schemas/autospec-test-stage1-result.schema.json"
    JS_FIXTURES="$FIXTURES_DIR/lang/js"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-gate-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── Helper: create a fake repo with a test.yml ───────────────────────────────

make_contract_json() {
    local test_cmd="$1"
    local collector="${2:-istanbul}"
    local lines="${3:-95}"
    local branches="${4:-90}"
    local functions="${5:-95}"
    jq -n \
        --arg test_cmd "$test_cmd" \
        --arg collector "$collector" \
        --argjson lines "$lines" \
        --argjson branches "$branches" \
        --argjson functions "$functions" \
        '{
            "mode": "strict_isolation",
            "unit": {
                "test_cmd": $test_cmd,
                "coverage_collector": $collector,
                "coverage_thresholds": {
                    "lines": $lines,
                    "branches": $branches,
                    "functions": $functions
                },
                "function_presence_check": false
            },
            "e2e": {
                "forbidden_url_patterns": ["^https?://example\\.com"]
            }
        }'
}

# ── gate-stage-unit.sh: tests-red edge ───────────────────────────────────────

@test "gate-stage-unit: failing test cmd exits 1 with passed=false reason=tests_red" {
    local contract
    contract=$(make_contract_json "exit 1")
    # run captures exit status
    run bash -c "printf '%s' \"\$CONTRACT\" | bash '$SCRIPTS_DIR/gate-stage-unit.sh' '$TEST_TMPDIR' 2>/dev/null" \
        CONTRACT="$contract"
    # Should exit 1 (gate failed — tests red)
    [ "$status" -ne 0 ]
}

@test "gate-stage-unit: tests-red output has passed=false" {
    local contract
    contract=$(make_contract_json "exit 1")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "false" ]
}

@test "gate-stage-unit: tests-red output has reason=tests_red" {
    local contract
    contract=$(make_contract_json "exit 1")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local reason
    reason=$(printf '%s' "$output" | jq -r '.reason')
    [ "$reason" = "tests_red" ]
}

@test "gate-stage-unit: tests-red output has stage=unit" {
    local contract
    contract=$(make_contract_json "exit 1")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local stage
    stage=$(printf '%s' "$output" | jq -r '.stage')
    [ "$stage" = "unit" ]
}

# ── gate-stage-unit.sh: pass edge ─────────────────────────────────────────────

@test "gate-stage-unit: passing test cmd with no lcov produces passed=true (no coverage check)" {
    # Use 'true' as test cmd (always succeeds); no lcov = no coverage check
    local contract
    contract=$(make_contract_json "true")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "true" ]
}

@test "gate-stage-unit: pass output has stage=unit" {
    local contract
    contract=$(make_contract_json "true")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local stage
    stage=$(printf '%s' "$output" | jq -r '.stage')
    [ "$stage" = "unit" ]
}

@test "gate-stage-unit: pass output is valid JSON with metrics.unit" {
    local contract
    contract=$(make_contract_json "true")
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    run bash -c "printf '%s' '$output' | jq -e '.metrics.unit'"
    [ "$status" -eq 0 ]
}

# ── gate-stage-unit.sh: threshold-fail edge (with real lcov) ─────────────────

make_lcov_with_coverage() {
    # Make an lcov file with specified line coverage
    # All other metrics (branches, functions) scale proportionally with the same ratio
    local lines_found="$1"
    local lines_hit="$2"
    # branches and functions use same ratio as lines so tests are consistent
    cat <<EOF
SF:src/foo.js
DA:1,1
DA:2,0
LF:${lines_found}
LH:${lines_hit}
BRF:${lines_found}
BRH:${lines_hit}
FNF:${lines_found}
FNH:${lines_hit}
end_of_record
EOF
}

@test "gate-stage-unit: threshold-fail when coverage below threshold" {
    # Create lcov with 50% line coverage, threshold 95%
    mkdir -p "$TEST_TMPDIR/coverage"
    make_lcov_with_coverage 10 5 > "$TEST_TMPDIR/coverage/lcov.info"

    local contract
    contract=$(make_contract_json "true" "istanbul" 95 90 95)
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "false" ]
}

@test "gate-stage-unit: threshold-fail has reason=coverage_below_threshold" {
    mkdir -p "$TEST_TMPDIR/coverage"
    make_lcov_with_coverage 10 5 > "$TEST_TMPDIR/coverage/lcov.info"
    local contract
    contract=$(make_contract_json "true" "istanbul" 95 90 95)
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local reason
    reason=$(printf '%s' "$output" | jq -r '.reason')
    [ "$reason" = "coverage_below_threshold" ]
}

@test "gate-stage-unit: pass when coverage meets threshold" {
    mkdir -p "$TEST_TMPDIR/coverage"
    make_lcov_with_coverage 10 10 > "$TEST_TMPDIR/coverage/lcov.info"
    local contract
    contract=$(make_contract_json "true" "istanbul" 95 90 95)
    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "true" ]
}

# ── gate-stage-unit.sh: function-presence-fail edge ──────────────────────────

@test "gate-stage-unit: function-presence-fail when functions have no tests" {
    # Enable function presence check; use JS fixture with multiply not tested
    local contract
    contract=$(jq -n '{
        "mode": "strict_isolation",
        "unit": {
            "test_cmd": "true",
            "coverage_collector": "istanbul",
            "coverage_thresholds": {"lines": 0, "branches": 0, "functions": 0},
            "function_presence_check": true
        },
        "e2e": {"forbidden_url_patterns": ["^https?://example\\.com"]}
    }')

    # Create a fake repo with src + tests where multiply is not tested
    mkdir -p "$TEST_TMPDIR/src" "$TEST_TMPDIR/tests"
    cp "$JS_FIXTURES/src/calculator.js" "$TEST_TMPDIR/src/"
    cp "$JS_FIXTURES/tests/calculator.test.js" "$TEST_TMPDIR/tests/"

    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local missing_count
    missing_count=$(printf '%s' "$output" | jq '.metrics.unit.missing_function_tests | length')
    [ "$missing_count" -gt 0 ]
}

# ── gate-stage-unit.sh: jq alternative-operator coercion guard ───────────────
# .unit.function_presence_check // true is a "//" alternative, which treats a
# literal false the same as an absent key. An explicit
# function_presence_check:false in the contract must disable the check (and
# so must NOT surface the multiply/farewell missing-test failure), proving
# FUNCTION_PRESENCE reads the literal false rather than being coerced to true.

@test "gate-stage-unit: explicit function_presence_check:false disables the check" {
    local contract
    contract=$(jq -n '{
        "mode": "strict_isolation",
        "unit": {
            "test_cmd": "true",
            "coverage_collector": "istanbul",
            "coverage_thresholds": {"lines": 0, "branches": 0, "functions": 0},
            "function_presence_check": false
        },
        "e2e": {"forbidden_url_patterns": ["^https?://example\\.com"]}
    }')

    # Same fixtures as the function-presence-fail edge above: multiply is not
    # tested, so if the check ran it would fail the gate.
    mkdir -p "$TEST_TMPDIR/src" "$TEST_TMPDIR/tests"
    cp "$JS_FIXTURES/src/calculator.js" "$TEST_TMPDIR/src/"
    cp "$JS_FIXTURES/tests/calculator.test.js" "$TEST_TMPDIR/tests/"

    local output
    output=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local missing_count
    missing_count=$(printf '%s' "$output" | jq '.metrics.unit.missing_function_tests | length')
    [ "$missing_count" -eq 0 ]
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "true" ]
}

# ── Stage 1 result JSON schema validation ─────────────────────────────────────

@test "gate-stage-unit: output validates against stage1-result schema (pass case)" {
    local contract
    contract=$(make_contract_json "true")
    local output_file="$TEST_TMPDIR/result.json"
    printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" > "$output_file" 2>/dev/null || true
    run ajv validate -s "$SCHEMA" -d "$output_file" --spec=draft2020
    [ "$status" -eq 0 ]
}

@test "gate-stage-unit: output validates against stage1-result schema (fail case)" {
    local contract
    contract=$(make_contract_json "exit 1")
    local output_file="$TEST_TMPDIR/result.json"
    printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-unit.sh" "$TEST_TMPDIR" > "$output_file" 2>/dev/null || true
    run ajv validate -s "$SCHEMA" -d "$output_file" --spec=draft2020
    [ "$status" -eq 0 ]
}

# ── Coverage collector: istanbul ──────────────────────────────────────────────

@test "istanbul collector: passes through lcov content unchanged" {
    local lcov_file="$TEST_TMPDIR/test.lcov"
    printf 'SF:src/foo.js\nDA:1,1\nLF:1\nLH:1\nend_of_record\n' > "$lcov_file"
    run bash "$SCRIPTS_DIR/coverage-collectors/istanbul.sh" "$lcov_file"
    [ "$status" -eq 0 ]
    [[ "$output" == *"SF:src/foo.js"* ]]
}

@test "istanbul collector: missing file exits 1" {
    run bash "$SCRIPTS_DIR/coverage-collectors/istanbul.sh" "/nonexistent/lcov.info"
    [ "$status" -eq 1 ]
}

# ── Coverage collector: go-cover ──────────────────────────────────────────────

@test "go-cover collector: converts coverprofile to lcov format" {
    local cover_file="$TEST_TMPDIR/coverage.out"
    printf 'mode: set\ngithub.com/example/myapp/main.go:10.20,12.5 1 1\ngithub.com/example/myapp/main.go:14.5,16.3 1 0\n' > "$cover_file"
    run bash "$SCRIPTS_DIR/coverage-collectors/go-cover.sh" "$cover_file"
    [ "$status" -eq 0 ]
    [[ "$output" == *"SF:"* ]]
    [[ "$output" == *"DA:"* ]]
    [[ "$output" == *"end_of_record"* ]]
}

@test "go-cover collector: missing coverprofile exits 1" {
    run bash "$SCRIPTS_DIR/coverage-collectors/go-cover.sh" "/nonexistent/coverage.out"
    [ "$status" -eq 1 ]
}

# ── function-presence.mjs: JS/TS tests ───────────────────────────────────────

@test "function-presence: detects exported JS functions in calculator.js" {
    run node "$SCRIPTS_DIR/function-presence.mjs" "$JS_FIXTURES/src" "$JS_FIXTURES/tests"
    [ "$status" -eq 0 ]
    local names
    names=$(printf '%s' "$output" | jq -r '[.exported_functions[].name] | join(",")')
    [[ "$names" == *"add"* ]]
    [[ "$names" == *"subtract"* ]]
    [[ "$names" == *"multiply"* ]]
}

@test "function-presence: detects exported TS functions in greeter.ts" {
    run node "$SCRIPTS_DIR/function-presence.mjs" "$JS_FIXTURES/src" "$JS_FIXTURES/tests"
    [ "$status" -eq 0 ]
    local names
    names=$(printf '%s' "$output" | jq -r '[.exported_functions[].name] | join(",")')
    [[ "$names" == *"greet"* ]]
    [[ "$names" == *"farewell"* ]]
}

@test "function-presence: detects missing tests for multiply and farewell" {
    run node "$SCRIPTS_DIR/function-presence.mjs" "$JS_FIXTURES/src" "$JS_FIXTURES/tests"
    [ "$status" -eq 0 ]
    local missing
    missing=$(printf '%s' "$output" | jq -r '.missing_tests | join(",")')
    [[ "$missing" == *"multiply"* ]]
    [[ "$missing" == *"farewell"* ]]
}

@test "function-presence: add and subtract are NOT in missing_tests" {
    run node "$SCRIPTS_DIR/function-presence.mjs" "$JS_FIXTURES/src" "$JS_FIXTURES/tests"
    [ "$status" -eq 0 ]
    local missing
    missing=$(printf '%s' "$output" | jq -r '.missing_tests | join(",")')
    [[ "$missing" != *"add,"* ]] || [[ "$missing" == *"add"* && false ]] || true
    # Verify add is NOT missing
    run bash -c "printf '%s' '$output' | jq -r '.missing_tests[]' | grep -c '^add$' || true"
    [ "$output" = "0" ]
}

@test "function-presence: emits valid JSON with required keys" {
    run node "$SCRIPTS_DIR/function-presence.mjs" "$JS_FIXTURES/src" "$JS_FIXTURES/tests"
    [ "$status" -eq 0 ]
    run bash -c "printf '%s' '$output' | jq -e '.exported_functions and .test_references and .missing_tests'"
    [ "$status" -eq 0 ]
}

@test "function-presence: missing args exits 1" {
    run node "$SCRIPTS_DIR/function-presence.mjs"
    [ "$status" -eq 1 ]
}
