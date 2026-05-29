#!/usr/bin/env bats
# tests/refine/test_refine_correctness.bats — autospec-refine correctness
# (issue #681). Covers:
#   1. --continue with real handoff fixture (mocked binary, but really invoked
#      per iteration — NOT --dry-run). Loop must terminate cleanly when the
#      harvested run-summary.md reports convergence.
#   2. Concurrent renders to the same output dir don't clobber each other
#      (mktemp atomics).
#   3. Handoff failure propagates: exit non-zero + status=handoff_failed +
#      handoff_executed=false + handoff_exit_code recorded.
#   4. PATH-stub in /tmp/ rejected unless AUTOSPEC_HANDOFF_DISPATCHER=1.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"
RENDER="${BATS_TEST_DIRNAME}/../../scripts/refine-render-overview.sh"

setup() {
    TEST_TMP="$(mktemp -d -t refine-correct.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs" "$REPO_ROOT/.autospec"
    ART_DIR="$TEST_TMP/artifacts"
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"

    FAKE_HOME="$TEST_TMP/home"
    mkdir -p "$FAKE_HOME/.autospec"
    export HOME="$FAKE_HOME"

    STUB_BIN="$TEST_TMP/bin"
    mkdir -p "$STUB_BIN"
    HANDOFF_LOG="$TEST_TMP/handoff.log"
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

# ───────────────────────────── 1. real --continue handoff ─────────────────
@test "issue #681: --continue invokes the real handoff per iteration (not --dry-run)" {
    # claude stub writes a run-summary.md announcing convergence so the loop
    # terminates cleanly on iteration 1 — but ONLY if the orchestrator
    # actually invoked the stub (the broken code path skipped the dispatcher
    # entirely under --continue).
    cat > "$STUB_BIN/claude" <<EOF
#!/usr/bin/env bash
echo "claude-invoked args=\$*" >> "$HANDOFF_LOG"
mkdir -p "$REPO_ROOT/.autospec"
cat > "$REPO_ROOT/.autospec/run-summary.md" <<'INNER'
# autospec run summary

## Next steps

- (none — converged)
INNER
exit 0
EOF
    chmod +x "$STUB_BIN/claude"
    export PATH="$STUB_BIN:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1  # stub lives outside tmpdir-deny on some platforms

    run bash "$SCRIPT" "fix login button" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    # The stub MUST have been invoked at least once — proves real handoff path.
    [ -f "$HANDOFF_LOG" ]
    grep -q "claude-invoked" "$HANDOFF_LOG"
    # Loop summary records convergence_clean.
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    [ -f "$LOOP_JSON" ]
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "convergence_clean" ]
}

@test "issue #681: --continue stops with iteration_error when real handoff fails" {
    cat > "$STUB_BIN/claude" <<EOF
#!/usr/bin/env bash
echo "claude-failed" >> "$HANDOFF_LOG"
exit 7
EOF
    chmod +x "$STUB_BIN/claude"
    export PATH="$STUB_BIN:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1

    run bash "$SCRIPT" "broken feature" --continue --max-iterations 3 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]  # outer continue-loop swallows iteration error into status field
    LOOP_JSON=$(ls "$ART_DIR"/*-loop.json | head -1)
    run jq -r '.status' "$LOOP_JSON"
    [ "$output" = "iteration_error" ]
}

# ───────────────────────────── 2. concurrent renders ──────────────────────
@test "issue #681 Finding 5: concurrent renders to same output dir don't clobber" {
    # Build a minimal valid input.json that satisfies the shape check.
    OUTDIR="$TEST_TMP/renders"
    mkdir -p "$OUTDIR"
    INPUT="$TEST_TMP/input.json"
    cat > "$INPUT" <<'EOF'
{
  "original_prompt": "p",
  "rounds": [],
  "final_prompt": "p",
  "status": "completed",
  "metadata": {
    "head_sha": "abc",
    "timestamp": "2026-05-28T00-00-00Z",
    "rounds_requested": 1,
    "rounds_executed": 0,
    "converged_early": false,
    "degraded_rounds": [],
    "handoff_target": "dry-run",
    "handoff_executed": false
  }
}
EOF

    # Fire two renders in parallel using distinct slugs so they target
    # different basenames but share OUTPUT_DIR. With predictable .tmp names
    # they would race on the same OUTPUT_DIR/refine-render.* mktemp pool;
    # mktemp guarantees unique temps per process.
    (bash "$RENDER" --json "$INPUT" --slug "alpha" --output-dir "$OUTDIR") &
    PID1=$!
    (bash "$RENDER" --json "$INPUT" --slug "beta" --output-dir "$OUTDIR") &
    PID2=$!
    wait "$PID1"
    R1=$?
    wait "$PID2"
    R2=$?
    [ "$R1" -eq 0 ]
    [ "$R2" -eq 0 ]
    # Both alpha and beta artifacts present, non-empty, no stale .tmp left.
    ls "$OUTDIR"/alpha-*.md >/dev/null
    ls "$OUTDIR"/beta-*.md  >/dev/null
    ls "$OUTDIR"/alpha-*.json >/dev/null
    ls "$OUTDIR"/beta-*.json  >/dev/null
    # No leftover mktemp scratch files.
    run bash -c "ls $OUTDIR/refine-render.* 2>/dev/null | wc -l | tr -d ' '"
    [ "$output" = "0" ]
}

# ───────────────────────────── 3. handoff failure propagates ──────────────
@test "issue #681 Finding 6: handoff failure propagates to exit code + status" {
    cat > "$STUB_BIN/claude" <<EOF
#!/usr/bin/env bash
exit 13
EOF
    chmod +x "$STUB_BIN/claude"
    export PATH="$STUB_BIN:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1

    run bash "$SCRIPT" "fix bug" --rounds 1 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 13 ]
    f=$(ls "$ART_DIR"/*.json | head -1)
    run jq -r '.status' "$f"
    [ "$output" = "handoff_failed" ]
    run jq -r '.metadata.handoff_executed' "$f"
    [ "$output" = "false" ]
    run jq -r '.metadata.handoff_exit_code' "$f"
    [ "$output" = "13" ]
}

@test "issue #681 Finding 6: handoff success records exit_code=0 + handoff_executed=true" {
    cat > "$STUB_BIN/claude" <<EOF
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$STUB_BIN/claude"
    export PATH="$STUB_BIN:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1

    run bash "$SCRIPT" "fix bug" --rounds 1 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    f=$(ls "$ART_DIR"/*.json | head -1)
    run jq -r '.metadata.handoff_executed' "$f"
    [ "$output" = "true" ]
    run jq -r '.metadata.handoff_exit_code' "$f"
    [ "$output" = "0" ]
}

# ───────────────────────────── 4. PATH-stub safety ────────────────────────
@test "issue #681 Finding 6: PATH-stub dispatcher in /tmp/ rejected without override" {
    # Place the stub in a tmpdir-like path. mktemp -d under /tmp ensures
    # the dispatcher resolves to /tmp/... or /private/tmp/... on macOS.
    TMPSTUB="$(mktemp -d -t refine-stub.XXXXXX)"
    cat > "$TMPSTUB/claude" <<EOF
#!/usr/bin/env bash
echo "STUB-RAN" > "$TEST_TMP/should-not-exist"
exit 0
EOF
    chmod +x "$TMPSTUB/claude"
    # Prepend the tmpdir stub so command -v claude resolves to it.
    OLDPATH="$PATH"
    export PATH="$TMPSTUB:$PATH"
    unset AUTOSPEC_HANDOFF_DISPATCHER

    run bash "$SCRIPT" "fix bug" --rounds 1 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    # Refusal exits non-zero (rc=5 from _handoff_dispatcher_safe).
    [ "$status" -ne 0 ]
    # Stub MUST NOT have been executed.
    [ ! -f "$TEST_TMP/should-not-exist" ]
    [[ "$output" == *"refusing handoff"* ]]
    rm -rf "$TMPSTUB"
    export PATH="$OLDPATH"
}

@test "issue #681 Finding 6: AUTOSPEC_HANDOFF_DISPATCHER=1 allows tmpdir dispatcher" {
    TMPSTUB="$(mktemp -d -t refine-stub-allowed.XXXXXX)"
    cat > "$TMPSTUB/claude" <<EOF
#!/usr/bin/env bash
echo "STUB-RAN" > "$TEST_TMP/stub-ran"
exit 0
EOF
    chmod +x "$TMPSTUB/claude"
    export PATH="$TMPSTUB:$PATH"
    export AUTOSPEC_HANDOFF_DISPATCHER=1

    run bash "$SCRIPT" "fix bug" --rounds 1 \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/stub-ran" ]
    rm -rf "$TMPSTUB"
}
