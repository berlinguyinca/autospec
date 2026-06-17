#!/usr/bin/env bats
# Tests for scripts/derive-trio.sh — single-source trio derivation (trio-derivation
# Phase 1). Covers the exact transform (codex = body frontmatter-stripped;
# opencode = own frontmatter + body), --check drift detection, --in-place
# idempotency, usage errors, and the migration-safety guard: --check must pass
# for EVERY existing skills/*/ trio (proving the generator already byte-matches
# today's hand-maintained mirrors).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/derive-trio.sh"
    FIXTURE="$REPO_ROOT/skills/autospec-explore"   # a representative real trio
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
}

# Materialize a writable copy of a real trio under $TMP so we can mutate it.
_copy_trio() {  # _copy_trio <src-skill-dir> <dest-dir>
    src="$1"; dest="$2"
    mkdir -p "$dest/codex" "$dest/opencode"
    cp "$src/SKILL.md" "$dest/SKILL.md"
    cp "$src/codex/prompt.md" "$dest/codex/prompt.md"
    cp "$src/opencode/agent.md" "$dest/opencode/agent.md"
}

# --- transform: --stdout codex == committed codex/prompt.md ---
@test "--stdout codex equals committed codex/prompt.md (frontmatter stripped)" {
    run bash "$SCRIPT" "$FIXTURE" --stdout codex
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" > "$TMP/codex.out"
    diff "$FIXTURE/codex/prompt.md" "$TMP/codex.out"
}

# --- transform: --stdout codex has NO YAML frontmatter ---
@test "--stdout codex emits no frontmatter" {
    run bash "$SCRIPT" "$FIXTURE" --stdout codex
    [ "$status" -eq 0 ]
    # first non-empty line must not be a --- frontmatter fence
    first="$(printf '%s\n' "$output" | sed '/^$/d' | head -1)"
    [ "$first" != "---" ]
    # the SKILL.md body content is present
    [[ "$output" == *"autospec-explore workflow"* ]]
}

# --- transform: --stdout opencode == committed opencode/agent.md ---
@test "--stdout opencode equals committed opencode/agent.md (own frontmatter + body)" {
    run bash "$SCRIPT" "$FIXTURE" --stdout opencode
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" > "$TMP/opencode.out"
    diff "$FIXTURE/opencode/agent.md" "$TMP/opencode.out"
}

# --- transform: opencode preserves its OWN frontmatter, not SKILL.md's ---
@test "--stdout opencode keeps opencode's own frontmatter (mode: primary)" {
    run bash "$SCRIPT" "$FIXTURE" --stdout opencode
    [ "$status" -eq 0 ]
    # opencode/agent.md frontmatter declares mode: primary; SKILL.md has name:.
    [[ "$output" == *"mode: primary"* ]]
}

# --- --check: in sync exits 0 ---
@test "--check exits 0 when mirrors are in sync" {
    run bash "$SCRIPT" "$FIXTURE" --check
    [ "$status" -eq 0 ]
}

# --- --check: codex drift exits non-zero with a diff ---
@test "--check exits non-zero and shows a diff when codex/prompt.md drifts" {
    _copy_trio "$FIXTURE" "$TMP/skill"
    printf '\nDRIFTED CODEX LINE\n' >> "$TMP/skill/codex/prompt.md"
    run bash "$SCRIPT" "$TMP/skill" --check
    [ "$status" -ne 0 ]
    [[ "$output" == *"DRIFTED CODEX LINE"* ]]
}

# --- --check: opencode drift exits non-zero ---
@test "--check exits non-zero when opencode/agent.md drifts" {
    _copy_trio "$FIXTURE" "$TMP/skill"
    printf '\nDRIFTED OPENCODE LINE\n' >> "$TMP/skill/opencode/agent.md"
    run bash "$SCRIPT" "$TMP/skill" --check
    [ "$status" -ne 0 ]
}

# --- --in-place: writes both mirrors and is idempotent ---
@test "--in-place repairs drift and a second run is a no-op (idempotent)" {
    _copy_trio "$FIXTURE" "$TMP/skill"
    printf '\nDRIFT\n' >> "$TMP/skill/codex/prompt.md"

    run bash "$SCRIPT" "$TMP/skill" --in-place
    [ "$status" -eq 0 ]
    # now in sync
    run bash "$SCRIPT" "$TMP/skill" --check
    [ "$status" -eq 0 ]

    # snapshot, run in-place again, must be byte-identical (idempotent)
    sum1_c="$(shasum -a 256 < "$TMP/skill/codex/prompt.md")"
    sum1_o="$(shasum -a 256 < "$TMP/skill/opencode/agent.md")"
    run bash "$SCRIPT" "$TMP/skill" --in-place
    [ "$status" -eq 0 ]
    sum2_c="$(shasum -a 256 < "$TMP/skill/codex/prompt.md")"
    sum2_o="$(shasum -a 256 < "$TMP/skill/opencode/agent.md")"
    [ "$sum1_c" = "$sum2_c" ]
    [ "$sum1_o" = "$sum2_o" ]
}

# --- --in-place: regenerates mirror bodies from SKILL.md while preserving the
#     opencode frontmatter; result matches the committed mirrors. codex is fully
#     derived (it has no frontmatter); opencode keeps its existing frontmatter,
#     so we corrupt only its BODY (the real-world case: edit SKILL.md, re-derive).
@test "--in-place regenerates mirror bodies and matches the committed mirrors" {
    _copy_trio "$FIXTURE" "$TMP/skill"
    # fully clobber codex (no frontmatter to preserve) and append drift to the
    # opencode BODY (its frontmatter stays intact as the preserved source).
    printf 'garbage\n' > "$TMP/skill/codex/prompt.md"
    printf '\nDRIFTED OPENCODE BODY\n' >> "$TMP/skill/opencode/agent.md"
    run bash "$SCRIPT" "$TMP/skill" --in-place
    [ "$status" -eq 0 ]
    diff "$FIXTURE/codex/prompt.md" "$TMP/skill/codex/prompt.md"
    diff "$FIXTURE/opencode/agent.md" "$TMP/skill/opencode/agent.md"
}

# --- usage: no args -> exit 2 ---
@test "no arguments exits 2" {
    run bash "$SCRIPT"
    [ "$status" -eq 2 ]
}

# --- usage: missing mode -> exit 2 ---
@test "skill dir without a mode exits 2" {
    run bash "$SCRIPT" "$FIXTURE"
    [ "$status" -eq 2 ]
}

# --- usage: non-directory -> exit 2 ---
@test "nonexistent skill dir exits 2" {
    run bash "$SCRIPT" "$TMP/does-not-exist" --check
    [ "$status" -eq 2 ]
}

# --- usage: bad member -> exit 2 ---
@test "unknown --stdout member exits 2" {
    run bash "$SCRIPT" "$FIXTURE" --stdout bogus
    [ "$status" -eq 2 ]
}

# --- usage: --help exits 0 and prints Usage ---
@test "--help prints Usage and exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

# --- MIGRATION-SAFETY GUARD: --check passes for EVERY existing trio ---
@test "--check passes for EVERY existing skills/*/ trio (migration-safety guard)" {
    local checked=0 failed=""
    for d in "$REPO_ROOT"/skills/*/; do
        [ -f "$d/SKILL.md" ] || continue
        [ -f "$d/codex/prompt.md" ] || continue
        [ -f "$d/opencode/agent.md" ] || continue
        checked=$((checked + 1))
        if ! bash "$SCRIPT" "$d" --check >/dev/null 2>&1; then
            failed="${failed} $(basename "$d")"
        fi
    done
    # there must be at least one real trio to make the guard meaningful
    [ "$checked" -gt 0 ]
    if [ -n "$failed" ]; then
        printf 'derive-trio --check drift for trios:%s\n' "$failed" >&2
    fi
    [ -z "$failed" ]
}
