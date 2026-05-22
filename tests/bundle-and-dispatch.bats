#!/usr/bin/env bats
# tests/bundle-and-dispatch.bats — TDD for skills/autospec-shared/scripts/bundle-and-dispatch.sh (issue #418)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-and-dispatch.sh"
BUNDLER="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/bundle-and-dispatch"

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

  # Fixture dynamic suffix file
  cat > "$FIXTURES_DIR/dynamic-suffix.txt" <<'EOF'
## Issue Body

Implement the feature described here.
Branch: feat/test-branch
EOF

  # Fixture memory-tags.yml (empty — no memory files needed for these tests)
  cat > "$FIXTURES_DIR/memory-tags.yml" <<'EOF'
EOF
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

@test "bundle-and-dispatch.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "bundle-and-dispatch.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "bundle-and-dispatch.sh exits 1 without required --role" {
  run "$SCRIPT" --issue-labels "ctx:64k" --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt"
  [ "$status" -ne 0 ]
}

@test "bundle-and-dispatch.sh exits 1 without required --dynamic-suffix-file" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k"
  [ "$status" -ne 0 ]
}

@test "bundle-and-dispatch.sh exits 1 if --dynamic-suffix-file does not exist" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" --dynamic-suffix-file "/nonexistent/path.txt"
  [ "$status" -ne 0 ]
}

@test "bundle-and-dispatch.sh output contains opening CACHE BOUNDARY marker" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "CACHE BOUNDARY"
}

@test "bundle-and-dispatch.sh output contains cached prefix from bundle-static-context.sh verbatim" {
  prefix=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$BUNDLER" --role implementer --issue-labels "ctx:64k")
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt"
  [ "$status" -eq 0 ]
  # Spot-check: a distinctive line from the prefix appears in combined output
  printf '%s\n' "$output" | grep -q "SKILL.md (implementer role)"
}

@test "bundle-and-dispatch.sh output appends dynamic suffix after cache boundary" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Implement the feature described here"
}

@test "bundle-and-dispatch.sh dynamic suffix appears after the closing CACHE BOUNDARY marker" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt"
  [ "$status" -eq 0 ]
  # Find line numbers of closing CACHE BOUNDARY and dynamic suffix content
  boundary_line=$(printf '%s\n' "$output" | grep -n "CACHE BOUNDARY" | tail -1 | cut -d: -f1)
  suffix_line=$(printf '%s\n' "$output" | grep -n "Implement the feature described here" | head -1 | cut -d: -f1)
  [ -n "$boundary_line" ]
  [ -n "$suffix_line" ]
  [ "$suffix_line" -gt "$boundary_line" ]
}

@test "bundle-and-dispatch.sh output is idempotent (two identical runs produce byte-identical stdout)" {
  out1=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt")
  out2=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "ctx:64k" \
    --dynamic-suffix-file "$FIXTURES_DIR/dynamic-suffix.txt")
  [ "$out1" = "$out2" ]
}

@test "SKILL.md orchestrator dispatch section references bundle-and-dispatch.sh" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  [ -f "$skill_md" ]
  grep -q "bundle-and-dispatch.sh" "$skill_md"
}

@test "codex/prompt.md orchestrator dispatch section references bundle-and-dispatch.sh" {
  codex_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
  [ -f "$codex_md" ]
  grep -q "bundle-and-dispatch.sh" "$codex_md"
}

@test "opencode/agent.md orchestrator dispatch section references bundle-and-dispatch.sh" {
  opencode_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
  [ -f "$opencode_md" ]
  grep -q "bundle-and-dispatch.sh" "$opencode_md"
}
