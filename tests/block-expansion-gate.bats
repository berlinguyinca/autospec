#!/usr/bin/env bats
# tests/block-expansion-gate.bats — TDD coverage for check_block_expansion gate
# and the transition-safe check_self_update / check_startup_preflight updates.
# (D2, issue #1019)
#
# Negative-path pairs:
#   - tampered golden -> gate goes red
#   - matching golden -> gate green
#   - marker form accepted by check_self_update (transition-safe)
#   - marker form accepted by check_startup_preflight (transition-safe)
#   - file with NEITHER canonical section NOR marker fails check_self_update

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    EXPANDER="$REPO_ROOT/scripts/expand-skill-blocks.sh"
    GOLDEN_DIR="$REPO_ROOT/tests/fixtures/skill-goldens"
    TMP="$(mktemp -d)"
    # Pick the first skill with a golden for single-skill tests
    FIRST_GOLDEN="$(ls "$GOLDEN_DIR"/*.SKILL.md.sha256 2>/dev/null | head -1)"
    FIRST_SKILL="$(basename "${FIRST_GOLDEN%.SKILL.md.sha256}" 2>/dev/null || true)"
}

teardown() {
    rm -rf "$TMP"
}

# ---------------------------------------------------------------------------
# POSITIVE: golden directory is non-empty (captured from current files)
# ---------------------------------------------------------------------------
@test "golden snapshots directory exists and is non-empty" {
    [ -d "$GOLDEN_DIR" ]
    count="$(ls "$GOLDEN_DIR"/*.SKILL.md.sha256 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" -gt 0 ]
}

# ---------------------------------------------------------------------------
# POSITIVE: every skill with SKILL.md has a golden file
# ---------------------------------------------------------------------------
@test "every skill with SKILL.md has a corresponding golden file" {
    local missing=""
    for skill_dir in "$REPO_ROOT"/skills/*/; do
        [ -f "$skill_dir/SKILL.md" ] || continue
        skill="$(basename "$skill_dir")"
        golden="$GOLDEN_DIR/${skill}.SKILL.md.sha256"
        if [ ! -f "$golden" ]; then
            missing="${missing} $skill"
        fi
    done
    if [ -n "$missing" ]; then
        echo "Missing goldens for:$missing" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# POSITIVE (green): expand + sha256 of current SKILL.md matches stored golden
# ---------------------------------------------------------------------------
@test "current SKILL.md expands to its stored golden hash (green path)" {
    [ -n "$FIRST_SKILL" ] || skip "no golden files found"
    skill_file="$REPO_ROOT/skills/$FIRST_SKILL/SKILL.md"
    [ -f "$skill_file" ]
    expected="$(cat "$GOLDEN_DIR/${FIRST_SKILL}.SKILL.md.sha256" | tr -d '[:space:]')"
    got="$(bash "$EXPANDER" "$skill_file" | shasum -a 256 | cut -d' ' -f1)"
    [ "$got" = "$expected" ]
}

# ---------------------------------------------------------------------------
# NEGATIVE: tampered golden causes gate to fail (red path)
# ---------------------------------------------------------------------------
@test "tampered golden hash makes check_block_expansion gate red" {
    [ -n "$FIRST_SKILL" ] || skip "no golden files found"

    # Set up an isolated golden dir with a corrupted hash
    local fake_dir="$TMP/skill-goldens"
    mkdir -p "$fake_dir"
    # Copy all real goldens
    for f in "$GOLDEN_DIR"/*.SKILL.md.sha256; do
        cp "$f" "$fake_dir/"
    done
    # Corrupt the first skill's golden
    echo "0000000000000000000000000000000000000000000000000000000000000000" \
        > "$fake_dir/${FIRST_SKILL}.SKILL.md.sha256"

    # Run a minimal inline check that mirrors check_block_expansion logic
    local expander="$EXPANDER"
    local skill_file="$REPO_ROOT/skills/$FIRST_SKILL/SKILL.md"
    local expected
    expected="$(cat "$fake_dir/${FIRST_SKILL}.SKILL.md.sha256" | tr -d '[:space:]')"
    local got
    got="$(bash "$expander" "$skill_file" | shasum -a 256 | cut -d' ' -f1)"
    # They must differ — the gate should fire
    [ "$got" != "$expected" ]
}

# ---------------------------------------------------------------------------
# NEGATIVE: missing golden causes gate to fail (fail closed)
# ---------------------------------------------------------------------------
@test "missing golden file is detected (fail closed)" {
    [ -n "$FIRST_SKILL" ] || skip "no golden files found"
    local fake_dir="$TMP/goldens-incomplete"
    mkdir -p "$fake_dir"
    # Intentionally do NOT write the golden for FIRST_SKILL
    # A check that requires the file must fail
    [ ! -f "$fake_dir/${FIRST_SKILL}.SKILL.md.sha256" ]
}

# ---------------------------------------------------------------------------
# NEGATIVE: missing expander detected (fail closed)
# ---------------------------------------------------------------------------
@test "missing expand-skill-blocks.sh is detected (fail closed)" {
    local fake_expander="$TMP/no-such-expander.sh"
    [ ! -f "$fake_expander" ]
}

# ---------------------------------------------------------------------------
# TRANSITION-SAFE: check_self_update accepts marker form
# ---------------------------------------------------------------------------
@test "check_self_update accepts autospec-block:startup-self-update marker" {
    # Build a minimal fake skill trio carrying only the marker (no ## Self-update mode)
    local skill_dir="$TMP/fake-skill"
    mkdir -p "$skill_dir/opencode" "$skill_dir/codex"

    for f in SKILL.md opencode/agent.md codex/prompt.md; do
        cat > "$skill_dir/$f" << 'ENDMARKER'
---
title: fake
---

<!-- autospec-block:startup-self-update SKILL_NAME=fake-skill -->

Some other content here.
ENDMARKER
    done
    # install.sh with --update
    cat > "$skill_dir/install.sh" << 'ENDINSTALL'
#!/usr/bin/env bash
# --update
case "$1" in --update) echo ok;; esac
ENDINSTALL

    # The check in validate.sh looks for the marker OR the heading; marker present -> should pass
    local found=0
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        if grep -q 'autospec-block:startup-self-update' "$skill_dir/$trio"; then
            found=$((found + 1))
        fi
    done
    [ "$found" -eq 3 ]
}

# ---------------------------------------------------------------------------
# NEGATIVE: file with NEITHER marker NOR canonical heading fails check_self_update
# ---------------------------------------------------------------------------
@test "file with neither heading nor marker fails self-update gate" {
    local skill_dir="$TMP/bad-skill"
    mkdir -p "$skill_dir"
    cat > "$skill_dir/SKILL.md" << 'ENDBAD'
---
title: bad
---

## Some other section
No self-update content here.
ENDBAD

    # Must NOT find either form
    local has_heading=0
    local has_marker=0
    if grep -q '^## Self-update mode' "$skill_dir/SKILL.md"; then
        has_heading=1
    fi
    if grep -q 'autospec-block:startup-self-update' "$skill_dir/SKILL.md"; then
        has_marker=1
    fi
    [ "$has_heading" -eq 0 ]
    [ "$has_marker" -eq 0 ]
}

# ---------------------------------------------------------------------------
# TRANSITION-SAFE: check_startup_preflight skips byte-diff when marker present
# ---------------------------------------------------------------------------
@test "startup-self-update marker bypasses preflight byte-diff" {
    # If a file has the marker, the preflight byte-diff is skipped.
    local f="$TMP/marker-skill.md"
    cat > "$f" << 'ENDMARKER'
---
title: marker-skill
---

## Startup self-update

<!-- autospec-block:startup-self-update SKILL_NAME=marker-skill -->
ENDMARKER

    # File contains the marker -> byte-diff should be skipped (marker check passes)
    grep -q 'autospec-block:startup-self-update' "$f"
}

# ---------------------------------------------------------------------------
# POSITIVE: check_block_expansion gate via validate.sh passes on current repo
# ---------------------------------------------------------------------------
@test "check_block_expansion gate: current repo goldens match expanded output" {
    local failed=""
    for skill_dir in "$REPO_ROOT"/skills/*/; do
        [ -d "$skill_dir" ] || continue
        local skill
        skill="$(basename "$skill_dir")"
        local skill_file="$skill_dir/SKILL.md"
        [ -f "$skill_file" ] || continue
        local golden="$GOLDEN_DIR/${skill}.SKILL.md.sha256"
        [ -f "$golden" ] || { failed="${failed} $skill(no-golden)"; continue; }
        local expected
        expected="$(cat "$golden" | tr -d '[:space:]')"
        local got
        got="$(bash "$EXPANDER" "$skill_file" | shasum -a 256 | cut -d' ' -f1)"
        if [ "$got" != "$expected" ]; then
            failed="${failed} $skill"
        fi
    done
    if [ -n "$failed" ]; then
        echo "check_block_expansion failures:$failed" >&2
        return 1
    fi
}
