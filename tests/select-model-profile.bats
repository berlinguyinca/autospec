#!/usr/bin/env bats
# tests/select-model-profile.bats — TDD for skills/autospec-run/scripts/select-model-profile.sh
# Covers routing of reasoning:shallow/medium → Haiku, reasoning:deep → sonnet.

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/select-model-profile"

setup() {
    mkdir -p "$FIXTURES_DIR"

    # Create a profiles file with both haiku and sonnet profiles
    PROFILES_FILE="$FIXTURES_DIR/model-profiles.yml"
    export AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE"

    cat > "$PROFILES_FILE" <<'EOF'
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 200000
  reasoning: medium
  allowed: ctx:small,ctx:medium,reasoning:shallow,reasoning:medium

claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64000
  reasoning: medium
  allowed: ctx:small,ctx:medium,reasoning:shallow,reasoning:medium
EOF
}

teardown() {
    rm -rf "$FIXTURES_DIR"
    unset AUTOSPEC_MODEL_PROFILES
    unset AUTOSPEC_TIER_B_PROFILE
}

@test "select-model-profile.sh is executable" {
    [ -x "$SCRIPT" ]
}

@test "select-model-profile.sh --help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

@test "reasoning:shallow routes to claude-haiku-cloud" {
    run bash "$SCRIPT" --labels "auto-implement,reasoning:shallow,ctx:64k"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-cloud" ]
}

@test "reasoning:medium routes to claude-haiku-cloud" {
    run bash "$SCRIPT" --labels "auto-implement,reasoning:medium,ctx:64k"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-cloud" ]
}

@test "reasoning:deep routes to claude-sonnet-cloud" {
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep,ctx:120k"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

@test "no reasoning label falls back to default (sonnet)" {
    run bash "$SCRIPT" --labels "auto-implement,area:hardening"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

@test "AUTOSPEC_TIER_B_PROFILE overrides default for reasoning:deep" {
    export AUTOSPEC_TIER_B_PROFILE="claude-opus-cloud"
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-opus-cloud" ]
}

@test "reasoning:shallow falls back to sonnet when haiku profile absent" {
    # Use a profiles file without haiku
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 200000
  reasoning: medium
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:shallow"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

@test "reasoning:medium falls back to sonnet when profiles file missing" {
    export AUTOSPEC_MODEL_PROFILES="/nonexistent/path/model-profiles.yml"
    run bash "$SCRIPT" --labels "auto-implement,reasoning:medium"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

# ── --print-model: resolve the profile, then emit its `model:` id ──────────────
# The dispatch site needs a concrete model id, not a profile name. Fail closed
# (exit 3, empty stdout) whenever no id can be resolved so the caller keeps
# TIER_B rather than guessing.

@test "--print-model emits the haiku model id for reasoning:shallow" {
    run bash "$SCRIPT" --labels "auto-implement,reasoning:shallow" --print-model
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-4-5" ]
}

@test "--print-model emits the sonnet model id for reasoning:deep" {
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "--print-model resolves a model id under a nested profiles: block" {
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "--print-model strips quotes from the model value" {
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
claude-sonnet-cloud:
  model: "claude-sonnet-4-6"
  ctx: 120k
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "--print-model fails closed when the profile has no model: key" {
    # Shape produced by autospec-run's auto-init: ctx/reasoning ceilings only.
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    ctx: 120k
    reasoning: deep
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "--print-model fails closed when the profiles file is missing" {
    export AUTOSPEC_MODEL_PROFILES="/nonexistent/path/model-profiles.yml"
    run bash "$SCRIPT" --labels "auto-implement,reasoning:medium" --print-model
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "--print-model does not leak a model: key from an adjacent profile" {
    # claude-haiku-cloud is resolved but has no model: of its own. The sonnet
    # block that follows must NOT be harvested.
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
claude-haiku-cloud:
  ctx: 64000
  reasoning: medium

claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 200000
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:shallow" --print-model
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "--print-model ignores a commented-out model: key" {
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
claude-sonnet-cloud:
  # model: claude-opus-4-7
  ctx: 120k
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

# ── Prose contract: all three harness surfaces must wire the selector ─────────
# The selector shipped for months with zero callers, so the Haiku trial never
# fired. These assertions are what keeps it wired.

@test "SKILL.md implementer dispatch resolves the model via select-model-profile.sh" {
    skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    [ -f "$skill_md" ]
    grep -q "select-model-profile.sh" "$skill_md"
    grep -q -- "--print-model" "$skill_md"
}

@test "codex/prompt.md implementer dispatch resolves the model via select-model-profile.sh" {
    codex_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
    [ -f "$codex_md" ]
    grep -q "select-model-profile.sh" "$codex_md"
    grep -q -- "--print-model" "$codex_md"
}

@test "opencode/agent.md implementer dispatch resolves the model via select-model-profile.sh" {
    opencode_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
    [ -f "$opencode_md" ]
    grep -q "select-model-profile.sh" "$opencode_md"
    grep -q -- "--print-model" "$opencode_md"
}

@test "auto-init sample profiles carry a model: key so the override is dispatchable" {
    skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    # A profiles file without model: makes --print-model exit 3 and the whole
    # override a silent no-op — the exact state found on a live host.
    grep -q "model: qwen3:32b" "$skill_md"
    grep -q "claude-haiku-cloud" "$skill_md"
}

@test "the reviewer dispatch does NOT take the implementer model override" {
    skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    # Invariant: reviewer tier >= implementer tier. If the override ever leaks
    # into the reviewer brief, a cheap model reviews its own tier's output.
    run grep -n "AUTOSPEC_REVIEWER_TIER" "$skill_md"
    [ "$status" -eq 0 ]
    reviewer_line=$(grep -n "AUTOSPEC_REVIEWER_TIER" "$skill_md" | head -1 | cut -d: -f1)
    # No --print-model wiring may appear in the reviewer tier paragraph.
    run sed -n "${reviewer_line}p" "$skill_md"
    [[ "$output" != *"--print-model"* ]]
}

@test "--print-model is unaffected by trailing inline comments" {
    cat > "$FIXTURES_DIR/model-profiles.yml" <<'EOF'
claude-sonnet-cloud:
  model: claude-sonnet-4-6   # pinned
  ctx: 120k
EOF
    run bash "$SCRIPT" --labels "auto-implement,reasoning:deep" --print-model
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}
