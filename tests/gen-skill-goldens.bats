#!/usr/bin/env bats
# Tests for scripts/gen-skill-goldens.sh — block-expansion golden regenerator
# (trio-derivation Phase 1). Proves the regenerated sha256 matches the expander
# one-liner that validate.sh's check_block_expansion runs, that a current run is
# a no-op (no diff), and that usage errors fail closed.
#
# NOTE: tests that mutate tests/fixtures/skill-goldens/ snapshot the affected
# file and restore it in teardown so the repo working tree is never left dirty.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/gen-skill-goldens.sh"
    EXPANDER="$REPO_ROOT/scripts/expand-skill-blocks.sh"
    GOLDEN_DIR="$REPO_ROOT/tests/fixtures/skill-goldens"
    SKILL="autospec-explore"
    GOLDEN="$GOLDEN_DIR/$SKILL.SKILL.md.sha256"
    BACKUP="$BATS_TEST_TMPDIR/golden.backup"
    cp "$GOLDEN" "$BACKUP"
}

teardown() {
    # always restore the snapshot so the working tree stays clean
    if [ -f "$BACKUP" ]; then
        cp "$BACKUP" "$GOLDEN"
    fi
}

# --- regenerated sha matches check_block_expansion's expander one-liner ---
@test "regenerated SKILL.md golden equals expander | shasum one-liner" {
    expected="$(bash "$EXPANDER" "$REPO_ROOT/skills/$SKILL/SKILL.md" | shasum -a 256 | cut -d' ' -f1)"
    # clobber, regenerate, compare
    printf 'deadbeef\n' > "$GOLDEN"
    run bash "$SCRIPT" "$SKILL"
    [ "$status" -eq 0 ]
    got="$(tr -d '[:space:]' < "$GOLDEN")"
    [ "$got" = "$expected" ]
}

# --- regenerated golden keeps the 64-hex + trailing-newline format ---
@test "regenerated golden is 64 hex chars + a trailing newline (65 bytes)" {
    printf 'deadbeef\n' > "$GOLDEN"
    run bash "$SCRIPT" "$SKILL"
    [ "$status" -eq 0 ]
    bytes="$(wc -c < "$GOLDEN" | tr -d '[:space:]')"
    [ "$bytes" -eq 65 ]
    # content (sans whitespace) is 64 lowercase hex chars
    hex="$(tr -d '[:space:]' < "$GOLDEN")"
    [ "${#hex}" -eq 64 ]
    [[ "$hex" =~ ^[0-9a-f]{64}$ ]]
}

# --- idempotent: a current golden is left untouched (no write) ---
@test "no-op when the golden is already current (no diff)" {
    # ensure current first
    bash "$SCRIPT" "$SKILL" >/dev/null
    before="$(shasum -a 256 < "$GOLDEN")"
    run bash "$SCRIPT" "$SKILL"
    [ "$status" -eq 0 ]
    # an already-current golden produces no "wrote" line
    [[ "$output" != *"wrote"* ]]
    after="$(shasum -a 256 < "$GOLDEN")"
    [ "$before" = "$after" ]
}

# --- whole-repo run is a clean no-op against the committed goldens ---
@test "full run prints no 'wrote' lines for an in-sync repo" {
    run bash "$SCRIPT"
    [ "$status" -eq 0 ]
    # If the committed goldens already match, the regenerator writes nothing.
    [[ "$output" != *"wrote"* ]]
}

# --- usage: unknown skill -> exit 3 (fail closed) ---
@test "unknown skill name exits 3" {
    run bash "$SCRIPT" definitely-not-a-real-skill
    [ "$status" -eq 3 ]
}

# --- usage: unknown option -> exit 2 ---
@test "unknown option exits 2" {
    run bash "$SCRIPT" --bogus
    [ "$status" -eq 2 ]
}

@test "shared skill smoke path delegates to golden regenerator" {
    run bash "$REPO_ROOT/skills/autospec-shared/scripts/gen-skill-goldens.sh" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

# --- usage: --help exits 0 and prints Usage ---
@test "--help prints Usage and exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}
