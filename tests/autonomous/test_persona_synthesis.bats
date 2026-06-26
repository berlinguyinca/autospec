#!/usr/bin/env bats
# tests/autonomous/test_persona_synthesis.bats
# TDD contract for scripts/autonomous-persona-synth.sh (F2).
#
# Coverage:
#   - Cadence/staleness gating: skip if fresh, synthesize if stale / --force
#   - Operator-edit block (<!-- operator-edited -->...<!-- /operator-edited -->)
#     preserved byte-identically across a refresh
#   - mkdir-based global lock prevents a concurrent clobber (exit 2)
#   - Stale lock is reclaimed (self-healing)
#   - Synthesis boundary mocked as a subprocess (AUTOSPEC_PERSONA_SYNTH_CMD)
#   - Structural presence check on output shape (no exact-text assertion)
#
# Engineering notes:
#   - All fixtures written to real temp files; no process substitutions with
#     [ -f ] (macOS bash 3.2: [ -f <(...) ] is always false)
#   - .autospec/ is gitignored; materialized at runtime
#   - No RETURN traps in the SUT; tests assert inline-cleanup behavior
#   - LLM/Agent boundary mocked as a subprocess via env seam

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
PERSONA_SYNTH="$SCRIPT_DIR/scripts/autonomous-persona-synth.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-synth.XXXXXX)"

    FAKE_REPO="$TMP/repo"
    mkdir -p "$FAKE_REPO/docs/memory"
    mkdir -p "$FAKE_REPO/.autospec"

    FAKE_AUTOSPEC="$TMP/autospec-home"
    mkdir -p "$FAKE_AUTOSPEC"

    GLOBAL_FILE="$FAKE_AUTOSPEC/operator-persona.md"
    LOCK_DIR="$FAKE_AUTOSPEC/.persona.lock.d"
    EFFECTIVE_FILE="$FAKE_REPO/.autospec/operator-persona.effective.md"

    # A well-shaped synthesis fixture for the mocked LLM seam to emit.
    MOCK_OUT="$TMP/mock-synth.md"
    {
        printf '# Operator persona\n\n'
        printf '## Decision style\n\nFavors correctness over speed.\n\n'
        printf '## Risk tolerance\n\nConservative; lock-step discipline.\n\n'
    } > "$MOCK_OUT"

    # Subprocess mock for the Tier-A synthesis boundary: drains stdin, emits the
    # fixture. This is a real executable subprocess, not an inline function.
    MOCK_CMD="$TMP/mock-synth.sh"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'cat >/dev/null\n'
        printf 'cat %q\n' "$MOCK_OUT"
    } > "$MOCK_CMD"
    chmod +x "$MOCK_CMD"

    export REPO_ROOT="$FAKE_REPO"
    export AUTOSPEC_HOME="$FAKE_AUTOSPEC"
    export AUTOSPEC_PERSONA_SYNTH_CMD="bash $MOCK_CMD"
}

teardown() {
    rm -rf "$TMP"
}

run_synth() {
    run bash "$PERSONA_SYNTH" "$@"
}

# ---------------------------------------------------------------------------
# Existence / basic hygiene
# ---------------------------------------------------------------------------

@test "script exists, is executable, and passes bash -n" {
    [ -f "$PERSONA_SYNTH" ]
    [ -x "$PERSONA_SYNTH" ]
    run bash -n "$PERSONA_SYNTH"
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Cadence / staleness
# ---------------------------------------------------------------------------

@test "stale (missing) global persona triggers synthesis" {
    [ ! -f "$GLOBAL_FILE" ]
    run_synth
    [ "$status" -eq 0 ]
    [ -f "$GLOBAL_FILE" ]
    grep -q '^# Operator persona' "$GLOBAL_FILE"
}

@test "fresh global persona skips re-synthesis" {
    # Materialize a fresh persona with a sentinel line.
    {
        printf '# Operator persona\n\n'
        printf '## Decision style\n\nSENTINEL-FRESH\n'
    } > "$GLOBAL_FILE"
    # mtime is "now" → within default 7-day window.

    export AUTOSPEC_PERSONA_REFRESH_DAYS=7
    run_synth
    [ "$status" -eq 0 ]
    # Untouched: sentinel still present.
    grep -q 'SENTINEL-FRESH' "$GLOBAL_FILE"
}

@test "stale global persona (older than window) is re-synthesized" {
    {
        printf '# Operator persona\n\n'
        printf '## Decision style\n\nSENTINEL-STALE\n'
    } > "$GLOBAL_FILE"
    # Age the file beyond a 1-day window.
    touch -t 202001010000 "$GLOBAL_FILE"

    export AUTOSPEC_PERSONA_REFRESH_DAYS=1
    run_synth
    [ "$status" -eq 0 ]
    # Re-synthesized: sentinel gone, mock content present.
    run grep -c 'SENTINEL-STALE' "$GLOBAL_FILE"
    [ "$output" = "0" ]
    grep -q 'Favors correctness over speed' "$GLOBAL_FILE"
}

@test "--force re-synthesizes even when fresh" {
    {
        printf '# Operator persona\n\n'
        printf '## Decision style\n\nSENTINEL-FRESH\n'
    } > "$GLOBAL_FILE"
    export AUTOSPEC_PERSONA_REFRESH_DAYS=7
    run_synth --force
    [ "$status" -eq 0 ]
    run grep -c 'SENTINEL-FRESH' "$GLOBAL_FILE"
    [ "$output" = "0" ]
}

@test "--dry-run never writes the global persona" {
    [ ! -f "$GLOBAL_FILE" ]
    run_synth --dry-run
    [ "$status" -eq 0 ]
    [ ! -f "$GLOBAL_FILE" ]
}

# ---------------------------------------------------------------------------
# Operator-edit preservation
# ---------------------------------------------------------------------------

@test "operator-edited block is preserved byte-identically across refresh" {
    # Seed an existing global persona with an operator-edited block.
    {
        printf '# Operator persona\n\n'
        printf '## Decision style\n\nold synthesized text\n\n'
        printf '<!-- operator-edited -->\n'
        printf 'HAND TUNED: always run Phase 5.5 broad audit.\n'
        printf '<!-- /operator-edited -->\n'
    } > "$GLOBAL_FILE"
    touch -t 202001010000 "$GLOBAL_FILE"

    # Capture the exact block bytes before refresh.
    BEFORE="$TMP/before-block.md"
    awk '/<!-- operator-edited -->/{k=1} k{print} /<!-- \/operator-edited -->/{k=0}' \
        "$GLOBAL_FILE" > "$BEFORE"

    export AUTOSPEC_PERSONA_REFRESH_DAYS=1
    run_synth --force
    [ "$status" -eq 0 ]

    AFTER="$TMP/after-block.md"
    awk '/<!-- operator-edited -->/{k=1} k{print} /<!-- \/operator-edited -->/{k=0}' \
        "$GLOBAL_FILE" > "$AFTER"

    # Byte-identical preservation.
    run diff "$BEFORE" "$AFTER"
    [ "$status" -eq 0 ]
    grep -q 'HAND TUNED: always run Phase 5.5 broad audit.' "$GLOBAL_FILE"
}

# ---------------------------------------------------------------------------
# Global-file lock
# ---------------------------------------------------------------------------

@test "live lock held by another holder blocks the write (exit 2)" {
    # Simulate a live holder: create the lock dir fresh (mtime = now).
    mkdir -p "$LOCK_DIR"
    printf '99999\n' > "$LOCK_DIR/pid"

    export AUTOSPEC_PERSONA_LOCK_STALE_MIN=30
    run_synth --force
    [ "$status" -eq 2 ]
    # The global file was NOT written.
    [ ! -f "$GLOBAL_FILE" ]
}

@test "stale lock is reclaimed and synthesis proceeds" {
    mkdir -p "$LOCK_DIR"
    printf '99999\n' > "$LOCK_DIR/pid"
    # Age the lock beyond the stale window.
    touch -t 202001010000 "$LOCK_DIR"

    export AUTOSPEC_PERSONA_LOCK_STALE_MIN=30
    run_synth --force
    [ "$status" -eq 0 ]
    [ -f "$GLOBAL_FILE" ]
    # Lock released inline after a successful run.
    [ ! -d "$LOCK_DIR" ]
}

@test "lock is released after a successful run (inline cleanup, no trap leak)" {
    run_synth
    [ "$status" -eq 0 ]
    [ ! -d "$LOCK_DIR" ]
}

# ---------------------------------------------------------------------------
# Structural shape + overlay
# ---------------------------------------------------------------------------

@test "structurally invalid synthesis output is rejected (exit 3)" {
    BAD_OUT="$TMP/bad.md"
    printf 'no heading here, just prose\n' > "$BAD_OUT"
    BAD_CMD="$TMP/bad-synth.sh"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'cat >/dev/null\n'
        printf 'cat %q\n' "$BAD_OUT"
    } > "$BAD_CMD"
    chmod +x "$BAD_CMD"
    export AUTOSPEC_PERSONA_SYNTH_CMD="bash $BAD_CMD"

    run_synth --force
    [ "$status" -eq 3 ]
    # No corrupt global file left behind.
    [ ! -f "$GLOBAL_FILE" ]
    # Lock released even on the failure path.
    [ ! -d "$LOCK_DIR" ]
}

@test "effective persona is produced with a per-dimension confidence note" {
    # Provide an overlay source so the bundle is non-empty.
    printf 'feedback content\n' > "$FAKE_REPO/docs/memory/feedback_demo.md"
    run_synth
    [ "$status" -eq 0 ]
    [ -f "$EFFECTIVE_FILE" ]
    grep -q '^## Confidence (per dimension)' "$EFFECTIVE_FILE"
    grep -q '^## Repo overlay' "$EFFECTIVE_FILE"
    # Confidence lines reference synthesized dimensions.
    grep -q 'Decision style:' "$EFFECTIVE_FILE"
}

@test "synthesis seam is invoked as a subprocess (mock receives stdin)" {
    # Mock that records that it was called and read stdin.
    MARKER="$TMP/seam-called"
    REC_CMD="$TMP/rec-synth.sh"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'cat >/dev/null\n'
        printf 'touch %q\n' "$MARKER"
        printf 'cat %q\n' "$MOCK_OUT"
    } > "$REC_CMD"
    chmod +x "$REC_CMD"
    export AUTOSPEC_PERSONA_SYNTH_CMD="bash $REC_CMD"

    run_synth
    [ "$status" -eq 0 ]
    [ -f "$MARKER" ]
}
