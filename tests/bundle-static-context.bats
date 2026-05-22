#!/usr/bin/env bats
# tests/bundle-static-context.bats — TDD for skills/autospec-shared/scripts/bundle-static-context.sh (issue #402)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/bundle-static-context"

setup() {
  mkdir -p "$FIXTURES_DIR/memory"
  mkdir -p "$FIXTURES_DIR/skills/autospec-run"
  mkdir -p "$FIXTURES_DIR/skills/autospec-shared/scripts"

  # Fixture SKILL.md for implementer role
  cat > "$FIXTURES_DIR/skills/autospec-run/SKILL.md" <<'EOF'
---
name: autospec-run
description: test fixture
---

# autospec-run workflow

This is the SKILL.md content for testing.
EOF

  # Fixture AGENTS.md with RULE_ID table
  cat > "$FIXTURES_DIR/AGENTS.md" <<'EOF'
# AGENTS.md

## Implementation-quality contract

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---------|----------|------|-------------------|
| `OUT_OF_SCOPE` | det | path-list compare | files touched not listed |
| `MISSING_TEST` | det | path-prefix scan | required test absent |

### Other section
EOF

  # Fixture memory files
  cat > "$FIXTURES_DIR/memory/feedback_autospec_run_prefs.md" <<'EOF'
---
tags: [autospec-run, monitor, orchestrator]
---
Always use ascending issue order in the monitor queue.
EOF

  cat > "$FIXTURES_DIR/memory/feedback_bash_style.md" <<'EOF'
---
tags: [bash, scripting]
---
Use set -eu at the top of every bash script.
EOF

  # Fixture memory-tags.yml
  cat > "$FIXTURES_DIR/memory-tags.yml" <<'EOF'
feedback_autospec_run_prefs.md:
  tags: [autospec-run, monitor, orchestrator]
feedback_bash_style.md:
  tags: [bash, scripting]
EOF
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

@test "bundle-static-context.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "bundle-static-context.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "bundle-static-context.sh --role implementer emits opening CACHE BOUNDARY marker" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | head -1 | grep -q "CACHE BOUNDARY"
}

@test "bundle-static-context.sh --role implementer emits closing CACHE BOUNDARY marker" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | tail -1 | grep -q "CACHE BOUNDARY"
}

@test "bundle-static-context.sh --role implementer output is idempotent" {
  local out1 out2
  out1=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run")
  out2=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run")
  [ "$out1" = "$out2" ]
}

@test "bundle-static-context.sh --role implementer includes AGENTS.md content" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Implementation-quality contract"
}

@test "bundle-static-context.sh --role implementer includes RULE_ID table" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "OUT_OF_SCOPE"
}

@test "bundle-static-context.sh --role implementer includes matching memory file" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "ascending issue order"
}

@test "bundle-static-context.sh --role implementer excludes non-matching memory file" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  # bash_style only matches "bash" or "scripting" — not "skill:autospec-run"
  ! printf '%s\n' "$output" | grep -q "set -eu at the top"
}

@test "bundle-static-context.sh exits 1 with unknown --role" {
  run "$SCRIPT" --role unknown-role
  [ "$status" -ne 0 ]
}

@test "SKILL.md Phase 4 implementer section references bundle-static-context.sh" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  [ -f "$skill_md" ]
  grep -q "bundle-static-context.sh" "$skill_md"
}
