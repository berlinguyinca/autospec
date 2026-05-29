#!/usr/bin/env bats
# tests/refine/test_refine_loop.bats — continuous-iteration mode (issue #673).
#
# Termination conditions covered:
#   1. convergence_clean          — harvested report has no next-steps content
#   2. oscillation_detected       — N+1 harvested prompt hash == N's
#   3. round_cap_reached          — --max-iterations cap hit
#   4. evidence_based_stop        — report contains STOP: marker
#   5. operator escape            — ~/.autospec/refine-loop-stop.flag triggers exit
#   6. budget_cap_reached         — AUTOSPEC_REFINE_LOOP_TOKEN_CAP exceeded
#
# Test hook: --simulate-iterations DIR. The script reads
# DIR/iter-<N>-report.md after refine+handoff for iteration N rather than
# spawning a real /autospec run.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"

setup() {
    TEST_TMP="$(mktemp -d -t refine-loop.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    ART_DIR="$TEST_TMP/artifacts"
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"
    SIM_DIR="$TEST_TMP/iterations"
    mkdir -p "$SIM_DIR"

    # Isolate operator stop flag location.
    FAKE_HOME="$TEST_TMP/home"
    mkdir -p "$FAKE_HOME/.autospec"
    export HOME="$FAKE_HOME"

    # Stub claude so handoff is silent.
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

@test "convergence_clean: empty next-steps list triggers convergence" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
# autospec run summary

## Next steps

- (none — converged)
EOF

    run bash "$SCRIPT" "fix login button" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    [ -f "$LOOP_JSON" ]
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "convergence_clean" ]
    LOOP_SUMMARY=$(ls "$ART_DIR"/*-loop-summary.md | head -1)
    [ -f "$LOOP_SUMMARY" ]
    grep -q 'convergence_clean' "$LOOP_SUMMARY"
}

@test "oscillation_detected: identical harvested prompt in iter 2 trips oscillation" {
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

@test "round_cap_reached: --max-iterations cap exits with status" {
    # Three iterations with always-fresh harvested prompts, cap at 2.
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

    run bash "$SCRIPT" "iterate" --continue --max-iterations 2 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "round_cap_reached" ]
    run jq -r '.iterations | length' "$LOOP_JSON"
    [ "$output" = "2" ]
}

@test "evidence_based_stop: STOP: marker terminates loop" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps

- Try one more thing

STOP: out-of-sample plateau evidence
EOF

    run bash "$SCRIPT" "benchmark" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "evidence_based_stop" ]
}

@test "operator escape: ~/.autospec/refine-loop-stop.flag terminates next iteration" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps

- Keep going indefinitely
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps

- Still more work
EOF
    # Pre-create the flag so iteration 2's boundary check trips.
    touch "$HOME/.autospec/refine-loop-stop.flag"

    run bash "$SCRIPT" "endless task" --continue --max-iterations 5 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "operator_stop" ]
}

@test "budget_cap_reached: token cap exit via --simulate-tokens" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps

- Continue
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps

- Continue more
EOF
    # Inject 1.5M tokens per iteration, cap at 1M.
    AUTOSPEC_REFINE_LOOP_TOKEN_CAP=1000000 \
    run bash "$SCRIPT" "big task" --continue --max-iterations 5 \
        --simulate-tokens 1500000 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "budget_cap_reached" ]
}

@test "loop-summary.md contains markdown table with iter column" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
## Next steps

- (none — converged)
EOF
    run bash "$SCRIPT" "task" --continue --max-iterations 2 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_SUMMARY=$(ls "$ART_DIR"/*-loop-summary.md | head -1)
    grep -q '| Iter ' "$LOOP_SUMMARY"
    grep -q 'Final status:' "$LOOP_SUMMARY"
}

@test "autospec trio carries canonical ## Next steps section" {
    # Lockstep guard: all three autospec trio files must document the section.
    REPO="${BATS_TEST_DIRNAME}/../.."
    grep -qE 'canonical `## Next steps` section' "$REPO/skills/autospec/SKILL.md" \
        || { echo "SKILL.md missing ## Next steps directive"; return 1; }
    grep -q 'Next steps' "$REPO/skills/autospec/codex/prompt.md" \
        || { echo "codex/prompt.md missing Next steps reference"; return 1; }
    grep -q 'Next steps' "$REPO/skills/autospec/opencode/agent.md" \
        || { echo "opencode/agent.md missing Next steps reference"; return 1; }
}
