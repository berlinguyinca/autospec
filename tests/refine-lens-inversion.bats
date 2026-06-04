#!/usr/bin/env bats
# tests/refine-lens-inversion.bats — D5 LLM-first inversion (issue #1024).
#
# Verifies the AUTOSPEC_REFINE_LENS_MODE env hatch (deterministic|llm|auto,
# default auto = LLM-first) and the flag-over-env precedence.
#
# All LLM calls go through PATH-shim stub binaries. The REAL claude/codex
# binaries are NEVER invoked. Per saved-memory recursion guard
# (feedback_path_shadow_mock_exec_recursion): the stubs are self-contained and
# do NOT `exec` the real binary, so prepending the stub dir to PATH cannot
# recurse. We also capture the real binary path BEFORE prepending so a stub
# could forward if it ever needed to.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/refine-prompt.sh"

setup() {
    TEST_TMP="$(mktemp -d -t refine-lens-inv.XXXXXX)"
    REPO_ROOT="$TEST_TMP/repo"
    mkdir -p "$REPO_ROOT/docs/specs"
    cat > "$REPO_ROOT/AGENTS.md" <<'EOF'
# AGENTS
Edit scripts/refine-prompt.sh and tests/refine/test_refine_lens_inversion.bats.
EOF
    MEMORY_ROOT="$TEST_TMP/memory"
    mkdir -p "$MEMORY_ROOT"
    ART_DIR="$TEST_TMP/artifacts"
    STUB_DIR="$TEST_TMP/stubs"
    mkdir -p "$STUB_DIR"

    # Recursion guard: capture the REAL binary path BEFORE prepending the stub
    # dir to PATH (feedback_path_shadow_mock_exec_recursion).
    REAL_CLAUDE="$(command -v claude || true)"
    export REAL_CLAUDE
    export AUTOSPEC_LLM_DISPATCHER=1
    # Ensure no leaked env hatch from the parent shell.
    unset AUTOSPEC_REFINE_LENS_MODE || true
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

make_success_stub() {
    # $1 = name (claude|codex), $2 = output text. Self-contained: prints a
    # marker and exits 0; never execs the real binary (no recursion).
    local name="$1"; local text="$2"
    cat > "$STUB_DIR/$name" <<EOF
#!/usr/bin/env bash
# stub $name — self-contained, no exec of real binary
printf '%s' "$text"
exit 0
EOF
    chmod +x "$STUB_DIR/$name"
}

@test "auto mode dispatches the LLM path first (no explicit --lens-mode)" {
    make_success_stub claude "AUTO-LLM-FIRST-OUTPUT"
    run bash "$SCRIPT" "do the thing" --rounds 1 --lenses repo-grounding --dry-run \
        --llm-binary "$STUB_DIR/claude" \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    [ "$(jq -r '.rounds[0].lens_implementation' "$f")" = "llm" ]
    [ "$(jq -r '.rounds[0].refined_prompt' "$f")" = "AUTO-LLM-FIRST-OUTPUT" ]
    [ "$(jq -r '.rounds[0].degraded_fallback' "$f")" = "false" ]
}

@test "auto mode falls back to deterministic + degraded_fallback=true when LLM unavailable" {
    # No llm-binary and no claude/codex on PATH → auto must fall back, not fail.
    # PATH keeps coreutils (/usr/bin:/bin) but excludes any claude/codex.
    run env -u AUTOSPEC_LLM_DISPATCHER PATH="$STUB_DIR:/usr/bin:/bin" \
        bash "$SCRIPT" "fix the login button" --rounds 1 --lenses clarity-ac --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    [ "$(jq -r '.rounds[0].lens_implementation' "$f")" = "deterministic" ]
    [ "$(jq -r '.rounds[0].degraded_fallback' "$f")" = "true" ]
    [[ "$(jq -r '.rounds[0].refined_prompt' "$f")" == *"Acceptance criteria"* ]]
}

@test "AUTOSPEC_REFINE_LENS_MODE=deterministic skips the LLM dispatcher entirely" {
    # A claude stub that EXPLODES if ever called proves no dispatch happened.
    cat > "$STUB_DIR/claude" <<'EOF'
#!/usr/bin/env bash
echo "LLM WAS CALLED — should not happen in deterministic mode" >&2
exit 42
EOF
    chmod +x "$STUB_DIR/claude"
    run env AUTOSPEC_REFINE_LENS_MODE=deterministic PATH="$STUB_DIR:$PATH" \
        bash "$SCRIPT" "do something" --rounds 1 --lenses repo-grounding --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    [ "$(jq -r '.rounds[0].lens_implementation' "$f")" = "deterministic" ]
    [ "$(jq -r '.rounds[0].degraded_fallback' "$f")" = "false" ]
}

@test "AUTOSPEC_REFINE_LENS_MODE=llm fails loudly when LLM is unavailable" {
    # llm mode = LLM-only; no dispatcher available → non-zero exit, no silent
    # deterministic fallback.
    run env -u AUTOSPEC_LLM_DISPATCHER AUTOSPEC_REFINE_LENS_MODE=llm PATH="$STUB_DIR:/usr/bin:/bin" \
        bash "$SCRIPT" "ship it" --rounds 1 --lenses repo-grounding --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -ne 0 ]
}

@test "invalid AUTOSPEC_REFINE_LENS_MODE value exits non-zero (allow-list)" {
    run env AUTOSPEC_REFINE_LENS_MODE=bogus \
        bash "$SCRIPT" "do the thing" --rounds 1 --lenses repo-grounding --dry-run \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -ne 0 ]
}

@test "invalid bare-mode exit per AC: empty stdin + bogus env exits non-zero" {
    # Mirrors the spec-level AC command form.
    run env AUTOSPEC_REFINE_LENS_MODE=bogus bash "$SCRIPT" </dev/null
    [ "$status" -ne 0 ]
}

@test "--lens-mode flag wins over AUTOSPEC_REFINE_LENS_MODE env (flag-over-env)" {
    # Env says deterministic, flag says llm — flag must win, so the LLM stub
    # fires and the round is tagged llm.
    make_success_stub claude "FLAG-WINS-LLM"
    run env AUTOSPEC_REFINE_LENS_MODE=deterministic \
        bash "$SCRIPT" "do the thing" --rounds 1 --lenses repo-grounding --dry-run \
        --lens-mode llm --llm-binary "$STUB_DIR/claude" \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    [ "$(jq -r '.rounds[0].lens_implementation' "$f")" = "llm" ]
    [ "$(jq -r '.rounds[0].refined_prompt' "$f")" = "FLAG-WINS-LLM" ]
}

@test "--lens-mode deterministic flag wins over env=llm (flag-over-env, both directions)" {
    # Env says llm, flag says deterministic — flag must win; no LLM dispatch.
    cat > "$STUB_DIR/claude" <<'EOF'
#!/usr/bin/env bash
echo "LLM WAS CALLED — flag should have forced deterministic" >&2
exit 42
EOF
    chmod +x "$STUB_DIR/claude"
    run env AUTOSPEC_REFINE_LENS_MODE=llm PATH="$STUB_DIR:$PATH" \
        bash "$SCRIPT" "do the thing" --rounds 1 --lenses repo-grounding --dry-run \
        --lens-mode deterministic \
        --artifact-dir "$ART_DIR" --repo-root "$REPO_ROOT" --memory-root "$MEMORY_ROOT"
    [ "$status" -eq 0 ]
    local f
    f="$(ls "$ART_DIR"/*.json | head -1)"
    [ "$(jq -r '.rounds[0].lens_implementation' "$f")" = "deterministic" ]
}

@test "AC: grep finds AUTOSPEC_REFINE_LENS_MODE hatch in refine-prompt.sh" {
    run grep -q 'AUTOSPEC_REFINE_LENS_MODE' "$SCRIPT"
    [ "$status" -eq 0 ]
}
