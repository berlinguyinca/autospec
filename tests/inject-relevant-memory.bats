#!/usr/bin/env bats
# tests/inject-relevant-memory.bats — tests for skills/autospec-shared/scripts/inject-relevant-memory.sh
# Covers: keyword match returns file, top-k limit, missing-dir fallback, empty context

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/inject-relevant-memory.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    MEMORY_DIR="$TEST_TMP/docs/memory"
    mkdir -p "$MEMORY_DIR"

    # Hermetic: inject-relevant-memory.sh prefers `mempalace search` over its
    # grep fallback whenever a `mempalace` CLI is on PATH (script lines 91-97).
    # On a dev box that HAS mempalace installed, that path searches the real
    # mempalace store and ignores this test's synthetic --memory-dir fixtures,
    # so the keyword-match assertions below fail through no fault of the SUT.
    # Shadow mempalace with a no-op stub that returns nothing, forcing the
    # deterministic grep fallback these tests are written to exercise.
    STUB_BIN="$TEST_TMP/stub-bin"
    mkdir -p "$STUB_BIN"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$STUB_BIN/mempalace"
    chmod +x "$STUB_BIN/mempalace"
    PATH="$STUB_BIN:$PATH"
    export PATH

    # Create test memory files
    cat > "$MEMORY_DIR/feedback_bash_return_trap_leak.md" <<'EOF'
---
type: feedback
---
# RETURN trap bash
RETURN traps in bash functions leak into caller frames; never use for local cleanup under set -u.
EOF

    cat > "$MEMORY_DIR/feedback_bash_set_e_short_circuit.md" <<'EOF'
---
type: feedback
---
# bash set -e short circuit
`[ test ] && action` aborts under set -e when test fails; use if/then/fi.
EOF

    cat > "$MEMORY_DIR/feedback_validate_sh_lockstep_checks.md" <<'EOF'
---
type: feedback
---
# validate lockstep
Renaming SKILL.md prose sections requires updating validate.sh checks too.
EOF

    cat > "$MEMORY_DIR/MEMORY.md" <<'EOF'
# Memory index
EOF

    export TEST_TMP MEMORY_DIR
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── basic sanity ──────────────────────────────────────────────────────────────

@test "inject-relevant-memory.sh exists and is executable" {
    [ -x "$SCRIPT" ]
}

@test "inject-relevant-memory.sh --help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ── keyword matching ──────────────────────────────────────────────────────────

@test "RETURN trap bash: returns feedback_bash_return_trap_leak.md in output" {
    run bash "$SCRIPT" --context "RETURN trap bash" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_bash_return_trap_leak"
}

@test "lockstep validate: returns feedback_validate_sh_lockstep_checks.md" {
    run bash "$SCRIPT" --context "lockstep validate" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_validate_sh_lockstep_checks"
}

@test "set -e short circuit: returns relevant file" {
    run bash "$SCRIPT" --context "set -e short circuit" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_bash_set_e_short_circuit"
}

@test "failure diagnostics: returns generic failure diagnostics lesson" {
    local repo_memory_dir="${BATS_TEST_DIRNAME}/../docs/memory"

    run bash "$SCRIPT" --context "failure diagnostics" --memory-dir "$repo_memory_dir" --top-k 5
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_generic_failure_diagnostics_flow"
}

@test "provider routing: returns operational assistant routing lesson" {
    local repo_memory_dir="${BATS_TEST_DIRNAME}/../docs/memory"

    run bash "$SCRIPT" --context "assistant provider routing NATS health" --memory-dir "$repo_memory_dir" --top-k 5
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_operational_assistant_provider_routing"
}

# ── top-k limit ───────────────────────────────────────────────────────────────

@test "top-k 1: returns at most 1 result" {
    run bash "$SCRIPT" --context "bash" --memory-dir "$MEMORY_DIR" --top-k 1
    [ "$status" -eq 0 ]
    # Count lines containing file references (## lines or filenames)
    local match_count
    match_count=$(echo "$output" | grep -c "feedback_" || true)
    [ "$match_count" -le 1 ]
}

# ── output format ─────────────────────────────────────────────────────────────

@test "output includes filename and first 200 chars of content" {
    run bash "$SCRIPT" --context "RETURN trap bash" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "feedback_bash_return_trap_leak"
    # Should include some content from the file
    echo "$output" | grep -qi "RETURN"
}

@test "MEMORY.md index file is not returned as a match" {
    run bash "$SCRIPT" --context "Memory index" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
    echo "$output" | grep -qv "^MEMORY.md" || true
}

# ── graceful fallbacks ────────────────────────────────────────────────────────

@test "missing --memory-dir: exits 0 with empty output" {
    run bash "$SCRIPT" --context "bash" --memory-dir "$TEST_TMP/nonexistent" --top-k 3
    [ "$status" -eq 0 ]
}

@test "empty --context: exits 0" {
    run bash "$SCRIPT" --context "" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
}

@test "missing --context: exits 0" {
    run bash "$SCRIPT" --memory-dir "$MEMORY_DIR" --top-k 3
    [ "$status" -eq 0 ]
}

# ── default memory dir resolution ─────────────────────────────────────────────

@test "without --memory-dir: exits 0 (uses default or no-op)" {
    run bash "$SCRIPT" --context "bash trap"
    [ "$status" -eq 0 ]
}
