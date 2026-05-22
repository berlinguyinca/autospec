#!/usr/bin/env bats
# tests/assemble-impl-prompt.bats — TDD for scripts/assemble-impl-prompt.sh (issue #391)

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/assemble-impl-prompt.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/assemble-impl-prompt"

setup() {
  mkdir -p "$FIXTURES_DIR/memory"

  # Fixture memory files with tags frontmatter
  cat > "$FIXTURES_DIR/memory/feedback_autospec_design_prefs.md" <<'EOF'
---
tags: [design, preferences, autospec, correctness, guardrails]
---
Always prefer correctness over speed. Use tight imperative triggers.
EOF

  cat > "$FIXTURES_DIR/memory/feedback_bash_return_trap_leak.md" <<'EOF'
---
tags: [bash, trap, set-u, functions, scripting]
---
RETURN traps in bash functions leak into caller frames; never use for local cleanup.
EOF

  # Fixture tag manifest
  cat > "$FIXTURES_DIR/memory-tags.yml" <<'EOF'
feedback_autospec_design_prefs.md:
  tags: [design, preferences, autospec, correctness, guardrails]
feedback_bash_return_trap_leak.md:
  tags: [bash, trap, set-u, functions, scripting]
EOF

  # Fixture AGENTS.md with RULE_ID table
  cat > "$FIXTURES_DIR/AGENTS.md" <<'EOF'
## Implementation-quality contract

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---------|----------|------|-------------------|
| `OUT_OF_SCOPE` | det | path-list compare | files touched not listed |
| `MISSING_TEST` | det | path-prefix scan | required test absent |
EOF
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

@test "assemble-impl-prompt.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "assemble-impl-prompt.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "assemble-impl-prompt.sh emits Project rules heading" {
  run "$SCRIPT" --issue-labels "autospec,bash" \
    --memory-dir "$FIXTURES_DIR/memory" \
    --manifest "$FIXTURES_DIR/memory-tags.yml" \
    --agents-md "$FIXTURES_DIR/AGENTS.md" \
    --ac-body "$FIXTURES_DIR/AGENTS.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "## Project rules you MUST honor"
}

@test "assemble-impl-prompt.sh includes memory file matching label tag" {
  run "$SCRIPT" --issue-labels "autospec,design" \
    --memory-dir "$FIXTURES_DIR/memory" \
    --manifest "$FIXTURES_DIR/memory-tags.yml" \
    --agents-md "$FIXTURES_DIR/AGENTS.md" \
    --ac-body "$FIXTURES_DIR/AGENTS.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "correctness over speed"
}

@test "assemble-impl-prompt.sh excludes memory file with no matching tag" {
  run "$SCRIPT" --issue-labels "bash,scripting" \
    --memory-dir "$FIXTURES_DIR/memory" \
    --manifest "$FIXTURES_DIR/memory-tags.yml" \
    --agents-md "$FIXTURES_DIR/AGENTS.md" \
    --ac-body "$FIXTURES_DIR/AGENTS.md"
  [ "$status" -eq 0 ]
  # bash tag matches bash_return_trap_leak but NOT design_prefs
  echo "$output" | grep -q "RETURN traps"
  # design_prefs should NOT appear (no matching tag)
  ! echo "$output" | grep -q "correctness over speed"
}

@test "assemble-impl-prompt.sh emits RULE_IDs heading" {
  run "$SCRIPT" --issue-labels "autospec" \
    --memory-dir "$FIXTURES_DIR/memory" \
    --manifest "$FIXTURES_DIR/memory-tags.yml" \
    --agents-md "$FIXTURES_DIR/AGENTS.md" \
    --ac-body "$FIXTURES_DIR/AGENTS.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "## RULE_IDs"
}

@test "assemble-impl-prompt.sh emits Acceptance criteria as constraints heading" {
  run "$SCRIPT" --issue-labels "autospec" \
    --memory-dir "$FIXTURES_DIR/memory" \
    --manifest "$FIXTURES_DIR/memory-tags.yml" \
    --agents-md "$FIXTURES_DIR/AGENTS.md" \
    --ac-body "$FIXTURES_DIR/AGENTS.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "## Acceptance criteria as constraints"
}

@test "SKILL.md Phase 4 template includes the three new section headings" {
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  [ -f "$skill_md" ]
  grep -q "## Project rules you MUST honor" "$skill_md"
  grep -q "## RULE_IDs" "$skill_md"
  grep -q "## Acceptance criteria as constraints" "$skill_md"
}
