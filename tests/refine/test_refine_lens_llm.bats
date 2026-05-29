#!/usr/bin/env bats
# tests/refine/test_refine_lens_llm.bats — LLM-driven lens dispatch (issue #684).
#
# All LLM calls are mocked via stub binaries on PATH. The real claude/codex
# binaries are NEVER invoked from these tests.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh"
LENS_LLM="${BATS_TEST_DIRNAME}/../../scripts/refine-prompt-lens-llm.sh"

setup() {
    TEST_TMP="$(mktemp -d -t refine-lens-llm.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    cat > "$REPO_ROOT/AGENTS.md" <<'EOF'
# AGENTS
Edit scripts/refine-prompt.sh and tests/refine/test_refine_lens_llm.bats.
EOF
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"
    ART_DIR="$TEST_TMP/artifacts"
    STUB_DIR="$TEST_TMP/stubs"
    mkdir -p "$STUB_DIR"
    # Stash original AUTOSPEC_LLM_DISPATCHER allowance.
    export AUTOSPEC_LLM_DISPATCHER=1
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

make_success_stub() {
    # $1 = name (claude|codex), $2 = output text
    local name="$1"; local text="$2"
    cat > "$STUB_DIR/$name" <<EOF
#!/usr/bin/env bash
# stub $name
printf '%s' "$text"
exit 0
EOF
    chmod +x "$STUB_DIR/$name"
}

make_failure_stub() {
    local name="$1"
    cat > "$STUB_DIR/$name" <<'EOF'
#!/usr/bin/env bash
echo "stub failure" >&2
exit 1
EOF
    chmod +x "$STUB_DIR/$name"
}

@test "each lens dispatches LLM call with mocked stdout" {
    make_success_stub claude "REFINED-BY-LLM-OUTPUT"

    for lens in repo-grounding clarity-ac sizing adversarial; do
        local art="$ART_DIR/$lens"
        run bash "$SCRIPT" "do the thing" --rounds 1 --lenses "$lens" --dry-run \
            --lens-mode llm --llm-binary "$STUB_DIR/claude" \
            --artifact-dir "$art" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
        [ "$status" -eq 0 ]
        local f
        f="$(ls "$art"/*.json | head -1)"
        local refined impl degraded
        refined="$(jq -r '.rounds[0].refined_prompt' "$f")"
        impl="$(jq -r '.rounds[0].lens_implementation' "$f")"
        degraded="$(jq -r '.rounds[0].degraded_fallback' "$f")"
        [ "$refined" = "REFINED-BY-LLM-OUTPUT" ]
        [ "$impl" = "llm" ]
        [ "$degraded" = "false" ]
    done
}

@test "LLM failure on a lens falls back to deterministic for that lens" {
    make_failure_stub claude

    run bash "$SCRIPT" "fix the login button" --rounds 1 --lenses clarity-ac --dry-run \
        --lens-mode llm --llm-binary "$STUB_DIR/claude" \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    local impl degraded refined
    impl="$(jq -r '.rounds[0].lens_implementation' "$f")"
    degraded="$(jq -r '.rounds[0].degraded_fallback' "$f")"
    refined="$(jq -r '.rounds[0].refined_prompt' "$f")"
    [ "$impl" = "deterministic" ]
    [ "$degraded" = "true" ]
    # Deterministic clarity-ac signature.
    [[ "$refined" == *"Acceptance criteria"* ]]
}

@test "lens_implementation metadata recorded per round" {
    make_success_stub claude "LLM-REFINED"

    run bash "$SCRIPT" "ship feature X" --rounds 3 --lenses repo-grounding,clarity-ac,sizing --dry-run \
        --lens-mode llm --llm-binary "$STUB_DIR/claude" \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    local count_llm
    count_llm="$(jq '[.rounds[] | select(.lens_implementation == "llm")] | length' "$f")"
    [ "$count_llm" -ge 1 ]
    # Every round has the field present.
    local missing
    missing="$(jq '[.rounds[] | select(has("lens_implementation") | not)] | length' "$f")"
    [ "$missing" = "0" ]
    local missing_dg
    missing_dg="$(jq '[.rounds[] | select(has("degraded_fallback") | not)] | length' "$f")"
    [ "$missing_dg" = "0" ]
}

@test "deterministic mode skips LLM dispatcher entirely" {
    # No stubs created — explicit deterministic mode must not call any binary.
    run bash "$SCRIPT" "do something" --rounds 1 --lenses repo-grounding --dry-run \
        --lens-mode deterministic \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    local impl
    impl="$(jq -r '.rounds[0].lens_implementation' "$f")"
    [ "$impl" = "deterministic" ]
}

@test "lens-llm dispatcher rejects tmpdir-resolved binary without override" {
    # Place a stub inside /tmp/ and confirm dispatcher refuses.
    local tmpstub
    tmpstub="$(mktemp -t fake-claude.XXXXXX)"
    printf '#!/usr/bin/env bash\necho ok\n' > "$tmpstub"
    chmod +x "$tmpstub"
    unset AUTOSPEC_LLM_DISPATCHER
    run bash "$LENS_LLM" --lens repo-grounding --prompt "hi" --llm-binary "$tmpstub"
    [ "$status" -eq 1 ]
    rm -f "$tmpstub"
}
