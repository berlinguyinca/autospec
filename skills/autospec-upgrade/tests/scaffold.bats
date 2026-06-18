#!/usr/bin/env bats
# scaffold.bats — structural tests for the autospec-upgrade trio skill scaffold.
# No network access, no installs. Pure filesystem + grep checks.

SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "SKILL.md exists" {
    [ -f "$SKILL_DIR/SKILL.md" ]
}

@test "codex/prompt.md exists" {
    [ -f "$SKILL_DIR/codex/prompt.md" ]
}

@test "opencode/agent.md exists" {
    [ -f "$SKILL_DIR/opencode/agent.md" ]
}

@test "install.sh exists" {
    [ -f "$SKILL_DIR/install.sh" ]
}

@test "uninstall.sh exists" {
    [ -f "$SKILL_DIR/uninstall.sh" ]
}

@test "README.md exists" {
    [ -f "$SKILL_DIR/README.md" ]
}

@test "install.sh references ~/.autospec/scripts" {
    grep -q '\.autospec/scripts' "$SKILL_DIR/install.sh"
}

@test "SKILL.md has correct skill name in frontmatter" {
    grep -q '^name: autospec-upgrade$' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md has SKILL_NAME=autospec-upgrade in autospec-block marker" {
    grep -q 'SKILL_NAME=autospec-upgrade' "$SKILL_DIR/SKILL.md"
}

@test "codex/prompt.md has no YAML frontmatter" {
    # codex/prompt.md must NOT start with ---
    first_line="$(head -1 "$SKILL_DIR/codex/prompt.md")"
    [ "$first_line" != "---" ]
}

@test "opencode/agent.md has YAML frontmatter" {
    first_line="$(head -1 "$SKILL_DIR/opencode/agent.md")"
    [ "$first_line" = "---" ]
}

@test "opencode/agent.md has name: autospec-upgrade" {
    grep -q '^name: autospec-upgrade$' "$SKILL_DIR/opencode/agent.md"
}

@test "scripts directory exists" {
    [ -d "$SKILL_DIR/scripts" ]
}
