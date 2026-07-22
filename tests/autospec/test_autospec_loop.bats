#!/usr/bin/env bats
# tests/autospec/test_autospec_loop.bats — shared loop-driver coverage for
# /autospec --loop (issue #708).
#
# Exercises scripts/lib/autospec-loop.sh via the refine-prompt.sh --continue
# entrypoint (which delegates to autospec_loop_run). Each test pins a
# termination condition via fixtures + env caps.
#
# Termination conditions covered:
#   1. convergence_clean   — harvest empty / (none — converged)
#   2. oscillation         — iter N+1 hash == iter N
#   3. round_cap_reached   — --max-iterations cap
#   4. evidence_based_stop — STOP: <reason> marker
#   5. operator_stop       — ~/.autospec/stop.flag present
#   6. budget_cap_reached  — AUTOSPEC_LOOP_TOKEN_CAP exceeded
#
# Plus: chemontology-style real-world scenario (mocked) where the loop
# successfully converges after 2 iterations.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"
LOOP_LIB="${BATS_TEST_DIRNAME}/../../scripts/lib/autospec-loop.sh"

setup() {
    TEST_TMP="$(mktemp -d -t autospec-loop.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    ART_DIR="$TEST_TMP/artifacts"
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"
    SIM_DIR="$TEST_TMP/iterations"
    mkdir -p "$SIM_DIR"

    FAKE_HOME="$TEST_TMP/home"
    mkdir -p "$FAKE_HOME/.autospec"
    export HOME="$FAKE_HOME"

    STUB_BIN="$TEST_TMP/bin"
    mkdir -p "$STUB_BIN"
    cat > "$STUB_BIN/claude" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$STUB_BIN/claude"
    export PATH="$STUB_BIN:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

@test "shared lib: scripts/lib/autospec-loop.sh exists and is bash -n clean" {
    [ -f "$LOOP_LIB" ]
    bash -n "$LOOP_LIB"
}

@test "shared lib: exposes autospec_loop_run + autospec_loop_harvest_next_prompt" {
    grep -q '^autospec_loop_run()' "$LOOP_LIB"
    grep -q '^autospec_loop_harvest_next_prompt()' "$LOOP_LIB"
}

@test "shared lib: refine-prompt.sh sources the shared loop driver" {
    grep -q 'lib/autospec-loop\.sh' "${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"
}

@test "shared lib: autospec-continue.sh sources the shared loop driver" {
    grep -q 'lib/autospec-loop\.sh' "${BATS_TEST_DIRNAME}/../../scripts/autospec-continue.sh"
}

@test "summary row preserves the supplied merged PR count" {
    run bash -c ". '$LOOP_LIB'; _autospec_loop_append_table_row '' 3 source.md next 7 convergence_clean"
    [ "$status" -eq 0 ]
    [[ "$output" =~ \|[[:space:]]+7[[:space:]]+\| ]]
}

@test "convergence_clean: empty Next steps → convergence_clean status" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
# autospec run summary
## Next steps

- (none — converged)
EOF
    run bash "$SCRIPT" "ship chemontology slice" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "convergence_clean" ]
}

@test "oscillation_detected: identical harvest in iter 2 trips oscillation" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps
- Add ChemOnt ontology layer
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps
- Add ChemOnt ontology layer
EOF
    run bash "$SCRIPT" "improve classifier" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "oscillation_detected" ]
}

@test "round_cap_reached: --max-iterations 2 caps at 2 fresh iters" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps
- Step alpha
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps
- Step beta
EOF
    cat > "$SIM_DIR/iter-3-report.md" <<'EOF'
## Next steps
- Step gamma
EOF
    run bash "$SCRIPT" "iterate work" --continue --max-iterations 2 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "round_cap_reached" ]
    run jq -r '.iterations | length' "$LOOP_JSON"
    [ "$output" = "2" ]
}

@test "evidence_based_stop: STOP: marker terminates with reason" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
# autospec run summary
STOP: spec contradicts existing schema; needs human review
EOF
    run bash "$SCRIPT" "fix login" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "evidence_based_stop" ]
}

@test "operator_stop: stop.flag triggers exit at iteration boundary" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps
- Continue with phase 2
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps
- Continue with phase 3
EOF
    # Plant stop flag BEFORE first iteration starts → loop should immediately
    # exit with operator_stop.
    touch "$FAKE_HOME/.autospec/stop.flag"
    run bash "$SCRIPT" "operator stop test" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "operator_stop" ]
}

@test "budget_cap_reached: AUTOSPEC_REFINE_LOOP_TOKEN_CAP exceeded" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps
- Step one
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps
- Step two
EOF
    cat > "$SIM_DIR/iter-3-report.md" <<'EOF'
## Next steps
- Step three
EOF
    # Each iteration burns 100 simulated tokens; cap at 150 → budget trips
    # after iter 2.
    AUTOSPEC_REFINE_LOOP_TOKEN_CAP=150 \
    run bash "$SCRIPT" "budget cap test" --continue --max-iterations 10 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR" --simulate-tokens 100
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "budget_cap_reached" ]
}

@test "chemontology scenario: 2 fresh iters then converge_clean" {
    # Mirrors the real-world chemontology campaign that motivated #708:
    # iter 1 = "Next best slice: ChemOnt ontology"
    # iter 2 = "Next best slice: NPClassifier integration"
    # iter 3 = converged (no more slices)
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
# autospec run summary
## Next steps
- Add ChemOnt ontology layer to classify_compound()
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
# autospec run summary
## Next steps
- Add NPClassifier integration to classify_compound()
EOF
    cat > "$SIM_DIR/iter-3-report.md" <<'EOF'
# autospec run summary
## Next steps
- (none — converged)
EOF
    run bash "$SCRIPT" "fix all chemontology slices" --continue --max-iterations 10 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "convergence_clean" ]
    # Iter 1 + Iter 2 + Iter 3 = 3 iteration records (the third is the
    # converged one — recorded even though it terminates the loop).
    run jq -r '.iterations | length' "$LOOP_JSON"
    [ "$output" = "3" ]
    # Summary table should mention convergence_clean.
    LOOP_MD=$(ls "$ART_DIR"/*-loop-summary.md | head -1)
    grep -q 'convergence_clean' "$LOOP_MD"
}

@test "loop summary markdown table is present" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps
- (none)
EOF
    run bash "$SCRIPT" "table test" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_MD=$(ls "$ART_DIR"/*-loop-summary.md | head -1)
    [ -f "$LOOP_MD" ]
    grep -q '^| Iter |' "$LOOP_MD"
    grep -q 'Final status:' "$LOOP_MD"
}
