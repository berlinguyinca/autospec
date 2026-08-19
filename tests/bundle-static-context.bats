#!/usr/bin/env bats
# tests/bundle-static-context.bats — TDD for skills/autospec-shared/scripts/bundle-static-context.sh (issue #402)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/bundle-static-context"

setup() {
  mkdir -p "$FIXTURES_DIR/memory"
  mkdir -p "$FIXTURES_DIR/skills/autospec-run/prompts"
  mkdir -p "$FIXTURES_DIR/skills/autospec-define/prompts"
  mkdir -p "$FIXTURES_DIR/skills/autospec-classify/prompts"
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

  # Fixture implementer-contract.md (D3: implementer role injects this, not SKILL.md)
  cat > "$FIXTURES_DIR/skills/autospec-run/prompts/implementer-contract.md" <<'EOF'
# Implementer contract (test fixture)

## Implementation-quality contract

### RULE_ID table

| RULE_ID | Detector |
|---|---|
| `OUT_OF_SCOPE` | det |
EOF

  # Fixture reviewer-contract.md (Phase 1: reviewer role injects this, not SKILL.md)
  cat > "$FIXTURES_DIR/skills/autospec-run/prompts/reviewer-contract.md" <<'EOF'
# Reviewer contract (test fixture)

## Guardian rubric

### RULE_ID table

| RULE_ID | Detector |
|---|---|
| `OUT_OF_SCOPE` | det |
| `MISSING_TEST` | det |

## Verdict

Return LGTM if clean.
EOF

  # Fixture decomposer-contract.md (M1: decomposer role injects this, not SKILL.md)
  cat > "$FIXTURES_DIR/skills/autospec-define/prompts/decomposer-contract.md" <<'EOF'
# Decomposer contract (test fixture)

## Phase 3 — Decompose into linked GitHub issues (delegate)

Sizing caps: body <=400 words, files touched <=3 logical units.
EOF

  # Fixture classifier-contract.md (M1: classifier role injects this, not SKILL.md)
  cat > "$FIXTURES_DIR/skills/autospec-classify/prompts/classifier-contract.md" <<'EOF'
# Classifier contract (test fixture)

## Rubric

ctx:32k / ctx:64k / ctx:120k; reasoning:shallow / medium / deep.
EOF

  # Fixture AGENTS.md with RULE_ID table + the sections the decomposer/classifier
  # roles extract (issue-quality, small-LLM target, subagent model selection).
  cat > "$FIXTURES_DIR/AGENTS.md" <<'EOF'
# AGENTS.md

## Implementation-quality contract

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---------|----------|------|-------------------|
| `OUT_OF_SCOPE` | det | path-list compare | files touched not listed |
| `MISSING_TEST` | det | path-prefix scan | required test absent |

### Other section

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs.

## Issue-quality contract

### Goal concreteness

The Goal section must contain exactly one sentence.

## Subagent model selection (two-tier, cost-aware)

Tier A is spec work; Tier B is implementation work.
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

@test "bundle-static-context.sh --role implementer emits two CACHE BOUNDARY markers (prefix is framed; only memory below the second)" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  boundary_count=$(printf '%s\n' "$output" | grep -c "CACHE BOUNDARY")
  [ "$boundary_count" -eq 2 ]
  # Counting markers alone would still pass with the whole bundle below them.
  # After the closing marker there must be exactly one section — saved memory.
  sections_below=$(printf '%s\n' "$output" \
    | awk '/CACHE BOUNDARY/ { c++; next } c >= 2 && /^## /' | grep -c '^## ')
  [ "$sections_below" -eq 1 ]
  printf '%s\n' "$output" | awk '/CACHE BOUNDARY/ { c++; next } c >= 2' \
    | grep -q '^## Project rules (saved memory)'
}

@test "bundle-static-context.sh --role implementer injects implementer-contract.md, NOT the SKILL.md" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  # The contract section header is injected for the implementer role...
  printf '%s\n' "$output" | grep -q "Implementer contract"
  # ...and the verbose SKILL.md prefix header is gone for the implementer role.
  ! printf '%s\n' "$output" | grep -q "SKILL.md (implementer role)"
}

@test "bundle-static-context.sh --role implementer keeps memory BELOW the closing CACHE BOUNDARY (byte-stable prefix)" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  # Second (closing) CACHE BOUNDARY line number vs the matched memory content.
  last_boundary=$(printf '%s\n' "$output" | grep -n "CACHE BOUNDARY" | tail -1 | cut -d: -f1)
  mem_line=$(printf '%s\n' "$output" | grep -n "ascending issue order" | head -1 | cut -d: -f1)
  [ -n "$last_boundary" ]
  [ -n "$mem_line" ]
  [ "$mem_line" -gt "$last_boundary" ]
}

# #3228: every fixed section belongs INSIDE the boundary. The lockstep paragraph
# and the role scaffolding are literal strings with no per-issue data, and they
# used to sit below the marker where they could not be cached.
@test "bundle-static-context.sh --role implementer keeps lockstep and scaffolding ABOVE the closing boundary" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  last_boundary=$(printf '%s\n' "$output" | grep -n "CACHE BOUNDARY" | tail -1 | cut -d: -f1)
  lock_line=$(printf '%s\n' "$output" | grep -n "^## Lockstep rules" | head -1 | cut -d: -f1)
  scaf_line=$(printf '%s\n' "$output" | grep -n "^## Implementer scaffolding" | head -1 | cut -d: -f1)
  [ -n "$lock_line" ]
  [ -n "$scaf_line" ]
  [ "$lock_line" -lt "$last_boundary" ]
  [ "$scaf_line" -lt "$last_boundary" ]
}

# #3228: --static-body carries the Phase 4 implementer template. It is one fixed
# file, so it must land inside the cached prefix; below the marker it cost ~8.9k
# uncached tokens on every dispatch and every retry.
@test "bundle-static-context.sh --static-body lands ABOVE the closing boundary" {
  bodyf="$BATS_TEST_TMPDIR/static-body.md"
  printf 'STATIC_BODY_SENTINEL\n' > "$bodyf"
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run" --static-body "$bodyf"
  [ "$status" -eq 0 ]
  last_boundary=$(printf '%s\n' "$output" | grep -n "CACHE BOUNDARY" | tail -1 | cut -d: -f1)
  body_line=$(printf '%s\n' "$output" | grep -n "STATIC_BODY_SENTINEL" | head -1 | cut -d: -f1)
  [ -n "$body_line" ]
  [ "$body_line" -lt "$last_boundary" ]
}

# The prefix is only cacheable if it stays byte-identical while labels vary. The
# body is inside it now, so this guards the body against label contamination too.
@test "bundle-static-context.sh --static-body keeps the prefix byte-stable across labels" {
  bodyf="$BATS_TEST_TMPDIR/static-body-stable.md"
  printf 'STATIC_BODY_SENTINEL\n' > "$bodyf"
  p1=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run" --static-body "$bodyf" \
    | awk '/<!-- CACHE BOUNDARY -->/{c++} {print} c==2{exit}')
  p2=$(env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "bash,scripting" --static-body "$bodyf" \
    | awk '/<!-- CACHE BOUNDARY -->/{c++} {print} c==2{exit}')
  [ "$p1" = "$p2" ]
  printf '%s\n' "$p1" | grep -q "STATIC_BODY_SENTINEL"
}

# A silently dropped body dispatches an implementer holding the contract but no
# procedure, which reads like a normal run.
@test "bundle-static-context.sh --static-body with a missing file exits non-zero" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --static-body "/nonexistent/static-body.md"
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -q "static-body file not found"
}

@test "bundle-static-context.sh --role implementer prefix is byte-stable across two different issue-label inputs" {
  pfx() {
    env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
      AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
      AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
      AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
      "$SCRIPT" --role implementer --issue-labels "$1" \
      | awk '/<!-- CACHE BOUNDARY -->/{c++} {print} c==2{exit}'
  }
  p1=$(pfx "skill:autospec-run")
  p2=$(pfx "bash,scripting")
  [ "$p1" = "$p2" ]
}

@test "implementer-contract.md exists and is at most 24576 bytes (size guard)" {
  contract="${BATS_TEST_DIRNAME}/../skills/autospec-run/prompts/implementer-contract.md"
  [ -f "$contract" ]
  size=$(wc -c < "$contract" | tr -d ' ')
  [ "$size" -le 24576 ]
}

@test "implementer-contract.md contains the single RULE_ID table header" {
  contract="${BATS_TEST_DIRNAME}/../skills/autospec-run/prompts/implementer-contract.md"
  [ -f "$contract" ]
  count=$(grep -c '^### RULE_ID table' "$contract")
  [ "$count" -eq 1 ]
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

@test "implementer bundle injects DESIGN.md when the repo has one" {
  printf '# Design Language\nprimary: #112233\nspacing: 8px grid\n' > "$FIXTURES_DIR/DESIGN.md"
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  rm -f "$FIXTURES_DIR/DESIGN.md"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q 'Design language (DESIGN.md'
  printf '%s\n' "$output" | grep -q '8px grid'
}

@test "implementer bundle omits the design section when no DESIGN.md exists" {
  rm -f "$FIXTURES_DIR/DESIGN.md"
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  ! printf '%s\n' "$output" | grep -q 'Design language (DESIGN.md'
}

@test "every role's scaffolding carries the Output discipline directive" {
  # Limited to roles whose SKILL the test fixture provides (autospec-run); the
  # directive is role-agnostic (emitted before the per-role case), so this is
  # representative.
  for role in implementer reviewer; do
    run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
      AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
      AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
      AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
      "$SCRIPT" --role "$role" --issue-labels "skill:autospec-run"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '## Output discipline'
    printf '%s\n' "$output" | grep -q 'minimal diffs'
  done
}

@test "bundle-static-context.sh output is cwd-independent (cross-directory invocation)" {
  # The script resolves all inputs from AUTOSPEC_REPO_ROOT, not $PWD. Run it once
  # from the tests directory (near the script) and once from an unrelated temp
  # working dir; output must be byte-identical and carry real content, proving the
  # script never reads relative to the caller's working directory.
  local workdir from_near from_elsewhere
  workdir="$BATS_TEST_TMPDIR/elsewhere"
  mkdir -p "$workdir"

  from_near=$(cd "$BATS_TEST_DIRNAME" && env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run")

  from_elsewhere=$(cd "$workdir" && env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run")

  [ -n "$from_near" ]
  [ "$from_near" = "$from_elsewhere" ]
  # Not merely "it ran": the cross-directory output carries the real bundle.
  printf '%s\n' "$from_elsewhere" | grep -q "CACHE BOUNDARY"
  printf '%s\n' "$from_elsewhere" | grep -q "Implementer contract"
}

# Corrected in #3228: this asserted the directive sat BELOW the boundary "so the
# prefix stays byte-stable". The premise was wrong — the directive is a fixed
# string, so it cannot vary by issue and cannot destabilize the prefix. Keeping it
# below the marker only made it uncacheable. The byte-stability it was protecting
# is asserted directly by the two prefix-stability cases above.
@test "Output discipline sits ABOVE the implementer cache boundary (fixed string, so cacheable)" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role implementer --issue-labels "skill:autospec-run"
  [ "$status" -eq 0 ]
  # The directive must appear BEFORE the second (closing) CACHE BOUNDARY marker.
  disc_line="$(printf '%s\n' "$output" | grep -n '## Output discipline' | head -1 | cut -d: -f1)"
  last_boundary="$(printf '%s\n' "$output" | grep -n 'CACHE BOUNDARY' | tail -1 | cut -d: -f1)"
  [ "$disc_line" -lt "$last_boundary" ]
}

@test "bundle-static-context.sh --role decomposer injects decomposer-contract.md, NOT the SKILL.md" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role decomposer --issue-labels "skill:autospec-define"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Decomposer contract"
  ! printf '%s\n' "$output" | grep -q "SKILL.md (decomposer role)"
}

@test "bundle-static-context.sh --role classifier injects classifier-contract.md, NOT the SKILL.md" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role classifier --issue-labels "skill:autospec-classify"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Classifier contract"
  ! printf '%s\n' "$output" | grep -q "SKILL.md (classifier role)"
}

@test "bundle-static-context.sh --role decomposer injects the AGENTS.md issue-quality + small-LLM sections" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role decomposer --issue-labels "skill:autospec-define"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "issue-quality + small-LLM target"
  printf '%s\n' "$output" | grep -q "Goal concreteness"
  printf '%s\n' "$output" | grep -q "32B-class local LLMs"
}

@test "bundle-static-context.sh --role classifier injects the AGENTS.md subagent-model-selection section" {
  run env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
    AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
    AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
    AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
    "$SCRIPT" --role classifier --issue-labels "skill:autospec-classify"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "subagent model selection"
  printf '%s\n' "$output" | grep -q "Tier A is spec work"
}

@test "decomposer-contract.md exists and is at most 20480 bytes (size guard)" {
  contract="${BATS_TEST_DIRNAME}/../skills/autospec-define/prompts/decomposer-contract.md"
  [ -f "$contract" ]
  size=$(wc -c < "$contract" | tr -d ' ')
  [ "$size" -le 20480 ]
}

@test "classifier-contract.md exists and is at most 9216 bytes (size guard)" {
  contract="${BATS_TEST_DIRNAME}/../skills/autospec-classify/prompts/classifier-contract.md"
  [ -f "$contract" ]
  size=$(wc -c < "$contract" | tr -d ' ')
  [ "$size" -le 9216 ]
}

@test "decomposer + classifier prefixes are byte-stable across issue-label inputs" {
  pfx() {
    env AUTOSPEC_REPO_ROOT="$FIXTURES_DIR" \
      AUTOSPEC_MEMORY_DIR="$FIXTURES_DIR/memory" \
      AUTOSPEC_SCRIPTS_DIR="${BATS_TEST_DIRNAME}/../scripts" \
      AUTOSPEC_MANIFEST="$FIXTURES_DIR/memory-tags.yml" \
      "$SCRIPT" --role "$1" --issue-labels "$2" \
      | awk '/<!-- CACHE BOUNDARY -->/{c++} {print} c==2{exit}'
  }
  [ "$(pfx decomposer 'skill:autospec-define')" = "$(pfx decomposer 'other,labels')" ]
  [ "$(pfx classifier 'skill:autospec-classify')" = "$(pfx classifier 'other,labels')" ]
}
