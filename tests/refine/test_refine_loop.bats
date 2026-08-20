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
    # Pin deterministic template lens — default is now auto/LLM-first (#1024),
    # but the loop's convergence/oscillation invariants assert deterministic output.
    export AUTOSPEC_REFINE_LENS_MODE=deterministic
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
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude
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

@test "tier 3.5 Next best slice: in report triggers continuation (issue #707)" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
# autospec run summary

Some prose.

Next best slice: extend the harvest matchers across both trios.
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps

- (none — converged)
EOF
    run bash "$SCRIPT" "fix harvest" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    # iter 1 should harvest non-empty (the next-best-slice prefix), iter 2 converges.
    run jq -r '.iterations[0].harvested_prompt' "$LOOP_JSON"
    [[ "$output" == *"extend the harvest matchers"* ]]
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "convergence_clean" ]
}

@test "tier 3.5 chemontology fixture: Next best slice: extracts body (issue #707)" {
    cat > "$SIM_DIR/iter-1-report.md" <<'EOF'
Next best slice: the 2 remaining Aromatic anilides rows, but only with blockers for pyridinecarboxamide and oligopeptide rows; otherwise move to Organic acids and derivatives > Carboxylic acids and derivatives.
EOF
    cat > "$SIM_DIR/iter-2-report.md" <<'EOF'
## Next steps

- (none — converged)
EOF
    run bash "$SCRIPT" "chemont" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT" \
        --simulate-iterations "$SIM_DIR"
    [ "$status" -eq 0 ]
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.iterations[0].harvested_prompt' "$LOOP_JSON"
    [[ "$output" == *"Aromatic anilides"* ]]
}

@test "the canonical ## Next steps directive is reachable from the autospec trio" {
    # #3262 turned /autospec into a router: the trio no longer documents Phase 6, so
    # requiring the directive in all three members would force that refactor to be
    # undone. run_autospec_refine_contract was updated to read the directive from
    # end-of-run.md; this suite was not, which is why it stayed red on main.
    #
    # Assert the directive at its single source, then assert each trio member reaches
    # the skill that owns it. Dropping the member assertions without the reachability
    # half would let the router stop delegating and nothing would notice.
    REPO="${BATS_TEST_DIRNAME}/../.."
    grep -qE 'canonical `## Next steps` section' \
        "$REPO/skills/autospec-run/references/end-of-run.md" \
        || { echo "end-of-run.md missing the canonical ## Next steps directive"; return 1; }
    for member in SKILL.md codex/prompt.md opencode/agent.md; do
        grep -qF 'skills/autospec-run/SKILL.md' "$REPO/skills/autospec/$member" \
            || { echo "skills/autospec/$member no longer delegates to autospec-run"; return 1; }
    done
}
