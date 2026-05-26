#!/usr/bin/env bats
# tests/unit/test_autospec_design.bats — scaffold-level tests for the
# autospec-design skill: presence of the 6 lockstep trio + install/uninstall
# + README files, lockstep equality (SKILL body == opencode body == codex
# body), and round-trip install/uninstall behavior across all three harnesses.
#
# Spec ref: docs/specs/2026-05-26-autospec-design-skill.md § Skill layout.
# Mirrors tests/unit/test_install_listener.bats.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec-design"
    INSTALL="$SKILL_DIR/install.sh"
    UNINSTALL="$SKILL_DIR/uninstall.sh"

    # Per-test fake HOME — so the real user environment never sees our writes
    # and tests run hermetically.
    FAKE_HOME="$(mktemp -d)"
    export HOME="$FAKE_HOME"
    export CLAUDE_CONFIG_DIR="$FAKE_HOME/.claude"
    export OPENCODE_CONFIG_DIR="$FAKE_HOME/.config/opencode"
    export CODEX_HOME="$FAKE_HOME/.codex"
    export AUTOSPEC_NO_SELF_UPDATE=1

    # Expected destinations (mirror install.sh).
    CLAUDE_DEST="$CLAUDE_CONFIG_DIR/skills/autospec-design/SKILL.md"
    OPENCODE_DEST="$OPENCODE_CONFIG_DIR/agent/autospec-design.md"
    CODEX_DEST="$CODEX_HOME/prompts/autospec-design.md"
}

teardown() {
    if [ -n "${FAKE_HOME:-}" ] && [ -d "$FAKE_HOME" ]; then
        rm -rf "$FAKE_HOME"
    fi
}

# ---- 6-file scaffold presence --------------------------------------------

@test "scaffold: SKILL.md exists" {
    [ -f "$SKILL_DIR/SKILL.md" ]
}

@test "scaffold: opencode/agent.md exists" {
    [ -f "$SKILL_DIR/opencode/agent.md" ]
}

@test "scaffold: codex/prompt.md exists" {
    [ -f "$SKILL_DIR/codex/prompt.md" ]
}

@test "scaffold: install.sh exists and is executable" {
    [ -f "$SKILL_DIR/install.sh" ]
    [ -x "$SKILL_DIR/install.sh" ]
}

@test "scaffold: uninstall.sh exists and is executable" {
    [ -f "$SKILL_DIR/uninstall.sh" ]
    [ -x "$SKILL_DIR/uninstall.sh" ]
}

@test "scaffold: README.md exists" {
    [ -f "$SKILL_DIR/README.md" ]
}

# ---- lockstep: SKILL body == opencode body == codex body -----------------

@test "lockstep: SKILL.md body (post-frontmatter) == codex/prompt.md verbatim" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    codex_body="$(cat "$SKILL_DIR/codex/prompt.md")"
    [ "$skill_body" = "$codex_body" ]
}

@test "lockstep: SKILL.md body (post-frontmatter) == opencode body (post-frontmatter)" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    opencode_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/opencode/agent.md")"
    [ "$skill_body" = "$opencode_body" ]
}

# ---- install --dry-run --harness all ---------------------------------------

@test "install --dry-run --harness all lists all destinations and exits 0" {
    run sh "$INSTALL" --harness all --dry-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "autospec-design"
}

# ---- install creates expected files --------------------------------------

@test "install creates expected files for --harness all" {
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

@test "install --harness claude only places the Claude file" {
    run sh "$INSTALL" --harness claude
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

@test "install --harness opencode only places the OpenCode file" {
    run sh "$INSTALL" --harness opencode
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

@test "install --harness codex only places the Codex prompt" {
    run sh "$INSTALL" --harness codex
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

# ---- round-trip install + uninstall --------------------------------------

@test "uninstall removes files placed by install (--harness all)" {
    sh "$INSTALL" --harness all >/dev/null
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]

    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

# ---- idempotency ---------------------------------------------------------

@test "install is idempotent (running twice in a row exits 0 both times)" {
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

@test "uninstall is idempotent (running twice in a row exits 0 both times)" {
    sh "$INSTALL" --harness all >/dev/null
    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
}

# ---- validate.sh wiring (issue #573) -------------------------------------

@test "validate.sh: check_startup_preflight enumerates autospec-design" {
    sed -n '/^check_startup_preflight()/,/^}/p' "$REPO_ROOT/scripts/validate.sh" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_codex_skills_install enumerates autospec-design" {
    sed -n '/^check_codex_skills_install()/,/^}/p' "$REPO_ROOT/scripts/validate.sh" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_shared_script_install enumerates autospec-design" {
    sed -n '/^check_shared_script_install()/,/^}/p' "$REPO_ROOT/scripts/validate.sh" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_subagent_model_tier has autospec-design case branch with expected_a=2 expected_b=0" {
    block="$(sed -n '/^check_subagent_model_tier()/,/^}/p' "$REPO_ROOT/scripts/validate.sh")"
    # Capture from "autospec-design)" up to and including the ";;" terminator.
    case_body="$(printf '%s\n' "$block" \
        | awk '/autospec-design)/{f=1} f{print; if (/;;/) exit}')"
    [ -n "$case_body" ]
    printf '%s\n' "$case_body" | grep -q 'expected_a=2'
    printf '%s\n' "$case_body" | grep -q 'expected_b=0'
}

@test "validate.sh: autospec-design appears in at least 4 hardcoded skill enumerations" {
    count="$(grep -c 'autospec-design' "$REPO_ROOT/scripts/validate.sh")"
    [ "$count" -ge 4 ]
}

@test "validate.sh: bash scripts/validate.sh exits 0 against current tree" {
    run bash "$REPO_ROOT/scripts/validate.sh"
    [ "$status" -eq 0 ]
}
