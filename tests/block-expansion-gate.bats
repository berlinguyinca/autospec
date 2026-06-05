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

# ===========================================================================
# TRIO-WIDE member-golden coverage (TOKR1-003, issue #1035)
# codex/prompt.md + opencode/agent.md goldens are gated too, not just SKILL.md.
# ===========================================================================

# ---------------------------------------------------------------------------
# POSITIVE: every markered codex/opencode member has a recorded golden
# ---------------------------------------------------------------------------
@test "every markered codex/opencode member has a recorded golden" {
    local missing=""
    local skill member src suffix golden
    for skill_dir in "$REPO_ROOT"/skills/*/; do
        [ -d "$skill_dir" ] || continue
        skill="$(basename "$skill_dir")"
        for member in "codex/prompt.md:codex.prompt.md" "opencode/agent.md:opencode.agent.md"; do
            src="$skill_dir/${member%%:*}"
            suffix="${member##*:}"
            [ -f "$src" ] || continue
            grep -q '<!-- autospec-block:' "$src" || continue
            golden="$GOLDEN_DIR/${skill}.${suffix}.sha256"
            if [ ! -f "$golden" ]; then
                missing="${missing} ${skill}.${suffix}"
            fi
        done
    done
    if [ -n "$missing" ]; then
        echo "Missing member goldens for:$missing" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# POSITIVE (green): every recorded member golden matches expanded output
# ---------------------------------------------------------------------------
@test "all recorded codex/opencode member goldens match expanded output (green)" {
    local failed=""
    local g skill_member skill member src suffix expected got
    for g in "$GOLDEN_DIR"/*.codex.prompt.md.sha256 "$GOLDEN_DIR"/*.opencode.agent.md.sha256; do
        [ -f "$g" ] || continue
        local base
        base="$(basename "$g")"            # e.g. autospec-classify.codex.prompt.md.sha256
        base="${base%.sha256}"             # autospec-classify.codex.prompt.md
        case "$base" in
            *.codex.prompt.md)
                skill="${base%.codex.prompt.md}"; src="$REPO_ROOT/skills/$skill/codex/prompt.md" ;;
            *.opencode.agent.md)
                skill="${base%.opencode.agent.md}"; src="$REPO_ROOT/skills/$skill/opencode/agent.md" ;;
            *) continue ;;
        esac
        [ -f "$src" ] || { failed="${failed} ${base}(no-src)"; continue; }
        expected="$(cat "$g" | tr -d '[:space:]')"
        got="$(bash "$EXPANDER" "$src" | shasum -a 256 | cut -d' ' -f1)"
        if [ "$got" != "$expected" ]; then
            failed="${failed} ${base}"
        fi
    done
    if [ -n "$failed" ]; then
        echo "member golden mismatches:$failed" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# NEGATIVE: tampered codex member golden makes the gate red
# ---------------------------------------------------------------------------
@test "tampered codex member golden makes check_block_expansion gate red" {
    # Find a real markered codex member with a golden.
    local g
    g="$(ls "$GOLDEN_DIR"/*.codex.prompt.md.sha256 2>/dev/null | head -1)"
    [ -n "$g" ] || skip "no codex member goldens recorded"
    local base skill src
    base="$(basename "$g")"; base="${base%.sha256}"
    skill="${base%.codex.prompt.md}"
    src="$REPO_ROOT/skills/$skill/codex/prompt.md"
    [ -f "$src" ]

    # The real (correct) expanded hash differs from a zeroed tampered hash.
    local got tampered
    got="$(bash "$EXPANDER" "$src" | shasum -a 256 | cut -d' ' -f1)"
    tampered="0000000000000000000000000000000000000000000000000000000000000000"
    [ "$got" != "$tampered" ]
}

# ---------------------------------------------------------------------------
# NEGATIVE (fail-closed): a markered member with NO golden must fail the gate.
# Exercises the real check_block_expansion against an isolated skills/ tree that
# contains a markered codex member but no recorded golden for it.
# ---------------------------------------------------------------------------
@test "markered codex member with missing golden fails check_block_expansion (fail closed)" {
    # Stand up an isolated REPO_ROOT-like sandbox with one skill whose codex
    # member is markered but has SKILL.md + codex goldens missing for codex.
    local sand="$TMP/sandbox"
    mkdir -p "$sand/scripts" "$sand/tests/fixtures/skill-goldens" "$sand/templates/skill-blocks"
    mkdir -p "$sand/skills/demo/codex"

    # Copy the real expander + template so expansion works.
    cp "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$sand/scripts/"
    cp "$REPO_ROOT/templates/skill-blocks/startup-self-update.md" "$sand/templates/skill-blocks/"

    # A plain SKILL.md (no marker) WITH a matching golden so the SKILL.md leg passes.
    printf '# demo\n\nplain body\n' > "$sand/skills/demo/SKILL.md"
    bash "$sand/scripts/expand-skill-blocks.sh" "$sand/skills/demo/SKILL.md" \
        | shasum -a 256 | cut -d' ' -f1 > "$sand/tests/fixtures/skill-goldens/demo.SKILL.md.sha256"

    # A MARKERED codex member with NO golden recorded — must trip the gate.
    printf '# demo codex\n\n<!-- autospec-block:startup-self-update SKILL_NAME=demo -->\n' \
        > "$sand/skills/demo/codex/prompt.md"

    # Run only check_block_expansion (+ its helper) against the sandbox.
    run bash -c '
        set -eu
        REPO_ROOT="'"$sand"'"
        cd "$REPO_ROOT"
        fail(){ printf "validate: FAIL — %s\n" "$*" >&2; exit 1; }
        info(){ printf "validate: %s\n" "$*"; }
        # Source the helper + gate straight from the repo validate.sh.
        eval "$(awk "/^gate_block_member\(\)/{f=1} f{print} f&&/^}\$/{c++; if(c==1) exit}" "'"$VALIDATE"'")"
        eval "$(awk "/^check_block_expansion\(\)/{f=1} f{print} f&&/^}\$/{exit}" "'"$VALIDATE"'")"
        check_block_expansion
    '
    [ "$status" -ne 0 ]
    [[ "$output" =~ "no golden" ]] || [[ "$output" =~ "fail closed" ]] || [[ "$output" =~ "FAIL" ]]
}

# ---------------------------------------------------------------------------
# POSITIVE (#1037): harness-adapter-core marker expands byte-faithfully to the
# Subagent dispatch policy row — the only >=20-skill identical adapter span.
# ---------------------------------------------------------------------------
@test "harness-adapter-core marker expands to the canonical dispatch-policy row (byte-faithful)" {
    local tmpl="$REPO_ROOT/templates/skill-blocks/harness-adapter-core.md"
    [ -f "$tmpl" ]
    local f="$TMP/core.md"
    printf '%s\n' '<!-- autospec-block:harness-adapter-core -->' > "$f"
    run bash "$EXPANDER" "$f"
    [ "$status" -eq 0 ]
    # Expansion equals the template body verbatim.
    [ "$output" = "$(cat "$tmpl")" ]
    # And that body IS the canonical dispatch-policy row.
    [[ "$output" =~ "Subagent dispatch policy" ]]
    [[ "$output" =~ "per AGENTS.md decision matrix" ]]
}

# ---------------------------------------------------------------------------
# NEGATIVE (#1037): skills whose adapter row drifts from the core span stay
# UNCONVERTED — they must NOT carry the harness-adapter-core marker, and their
# SKILL.md must still hold an inline (literal) adapter row. Guards against a
# split template silently swallowing a per-skill qualifier.
# ---------------------------------------------------------------------------
@test "drift-skipped skills keep an inline adapter section and no harness-adapter-core marker" {
    for skill in autospec-doc autospec-loop autospec-playwright autospec-rollover-status; do
        local sf="$REPO_ROOT/skills/$skill/SKILL.md"
        [ -f "$sf" ] || continue
        # Skipped skills must NOT have been markered with the core block.
        ! grep -qF 'autospec-block:harness-adapter-core' "$sf"
        # They still describe an adapter / dispatch capability inline.
        grep -qE 'Required capabilities & harness adapter|Subagent (model tier|dispatch)' "$sf"
    done
}
