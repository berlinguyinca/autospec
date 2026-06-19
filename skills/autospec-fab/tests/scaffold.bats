#!/usr/bin/env bats
# scaffold.bats — structural integrity tests for the autospec-fab skill trio.
# No network access, no installs required.

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

@test "SKILL.md frontmatter name == autospec-fab" {
    # Extract the name field from YAML frontmatter (between first two --- lines)
    name="$(awk '/^---$/{c++; if(c==2) exit} c==1 && /^name:/{print $2}' "$SKILL_DIR/SKILL.md")"
    [ "$name" = "autospec-fab" ]
}

@test "SKILL.md autospec-block has SKILL_NAME=autospec-fab" {
    grep -q 'SKILL_NAME=autospec-fab' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains STL Modeling Rules" {
    grep -q 'STL Modeling Rules' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains change workflow" {
    grep -q 'change workflow\|Change workflow\|change-workflow' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains composition map" {
    grep -q 'omposition map\|Composition map' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains watertight" {
    grep -q 'watertight' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains gasket" {
    grep -q 'gasket' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains vacuum" {
    grep -q 'vacuum\|Vacuum' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains FreeCAD" {
    grep -q 'FreeCAD' "$SKILL_DIR/SKILL.md"
}

@test "SKILL.md contains release-gate" {
    grep -q 'release-gate\|release gate\|Release gate' "$SKILL_DIR/SKILL.md"
}

@test "codex/prompt.md contains SKILL_NAME=autospec-fab" {
    grep -q 'SKILL_NAME=autospec-fab' "$SKILL_DIR/codex/prompt.md"
}

@test "opencode/agent.md contains SKILL_NAME=autospec-fab" {
    grep -q 'SKILL_NAME=autospec-fab' "$SKILL_DIR/opencode/agent.md"
}

@test "opencode/agent.md has frontmatter with name: autospec-fab" {
    name="$(awk '/^---$/{c++; if(c==2) exit} c==1 && /^name:/{print $2}' "$SKILL_DIR/opencode/agent.md")"
    [ "$name" = "autospec-fab" ]
}
