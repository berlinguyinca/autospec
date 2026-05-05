#!/usr/bin/env bats
# tests/smoke/test_install_all_skills.bats — round-trip the top-level
# install.sh --skill all into a per-test tmpdir HOME and assert that all
# all suite skill bodies land for every harness, and uninstall removes them.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.2, §6.3.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    INSTALL="$REPO_ROOT/install.sh"
    UNINSTALL="$REPO_ROOT/uninstall.sh"

    # Per-test fake HOME to keep the real env untouched.
    FAKE_HOME="$(mktemp -d)"
    export HOME="$FAKE_HOME"
    export CLAUDE_CONFIG_DIR="$FAKE_HOME/.claude"
    export OPENCODE_CONFIG_DIR="$FAKE_HOME/.config/opencode"
    export CODEX_HOME="$FAKE_HOME/.codex"

    SKILLS="autospec autospec-split autospec-define autospec-run autospec-classify autospec-listen autospec-stop"
}

teardown() {
    if [ -n "${FAKE_HOME:-}" ] && [ -d "$FAKE_HOME" ]; then
        rm -rf "$FAKE_HOME"
    fi
}

# Helpers.
_claude_path() { printf '%s/.claude/skills/%s/SKILL.md' "$FAKE_HOME" "$1"; }
_opencode_path() { printf '%s/.config/opencode/agent/%s.md' "$FAKE_HOME" "$1"; }
_codex_path() { printf '%s/.codex/prompts/%s.md' "$FAKE_HOME" "$1"; }

# ---- install all skills --------------------------------------------------

@test "install --skill all places every suite skill body in each harness" {
    run bash "$INSTALL" --skill all --harness all
    [ "$status" -eq 0 ]
    for s in $SKILLS; do
        [ -f "$(_claude_path "$s")" ]
        [ -f "$(_opencode_path "$s")" ]
        [ -f "$(_codex_path "$s")" ]
    done
}

# ---- uninstall cleans up -------------------------------------------------

@test "uninstall --skill all cleans up every harness file" {
    bash "$INSTALL" --skill all --harness all >/dev/null
    # Sanity: at least one was placed.
    [ -f "$(_claude_path autospec)" ]

    run bash "$UNINSTALL" --skill all --harness all
    [ "$status" -eq 0 ]
    for s in $SKILLS; do
        [ ! -f "$(_claude_path "$s")" ]
        [ ! -f "$(_opencode_path "$s")" ]
        [ ! -f "$(_codex_path "$s")" ]
    done
}

# ---- idempotency ---------------------------------------------------------

@test "install --skill all is idempotent (running twice exits 0 both times)" {
    run bash "$INSTALL" --skill all --harness all
    [ "$status" -eq 0 ]
    run bash "$INSTALL" --skill all --harness all
    [ "$status" -eq 0 ]
    for s in $SKILLS; do
        [ -f "$(_claude_path "$s")" ]
        [ -f "$(_opencode_path "$s")" ]
        [ -f "$(_codex_path "$s")" ]
    done
}
