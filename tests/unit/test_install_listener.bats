#!/usr/bin/env bats
# tests/unit/test_install_listener.bats — round-trip install + uninstall
# of the autospec-listen skill into a per-test tmpdir HOME and assert that
# files are placed/removed correctly across all three harnesses.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.2, §6.3.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec-listen"
    INSTALL="$SKILL_DIR/install.sh"
    UNINSTALL="$SKILL_DIR/uninstall.sh"

    # Per-test fake HOME — so the real user environment never sees our
    # writes and tests run hermetically.
    FAKE_HOME="$(mktemp -d)"
    export HOME="$FAKE_HOME"
    export CLAUDE_CONFIG_DIR="$FAKE_HOME/.claude"
    export OPENCODE_CONFIG_DIR="$FAKE_HOME/.config/opencode"
    export CODEX_HOME="$FAKE_HOME/.codex"

    # Expected destinations (mirror install.sh).
    CLAUDE_DEST="$CLAUDE_CONFIG_DIR/skills/autospec-listen/SKILL.md"
    OPENCODE_DEST="$OPENCODE_CONFIG_DIR/agent/autospec-listen.md"
    CODEX_DEST="$CODEX_HOME/prompts/autospec-listen.md"
}

teardown() {
    if [ -n "${FAKE_HOME:-}" ] && [ -d "$FAKE_HOME" ]; then
        rm -rf "$FAKE_HOME"
    fi
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

# ---- uninstall removes the files -----------------------------------------

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
