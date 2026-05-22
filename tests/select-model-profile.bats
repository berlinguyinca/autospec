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
