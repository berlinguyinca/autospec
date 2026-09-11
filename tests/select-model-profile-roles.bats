#!/usr/bin/env bats
# tests/select-model-profile-roles.bats — Guardrail R2: role-aware model profiles.
#
# A profile may declare which dispatch kinds it may serve via a `roles:` list.
# `select-model-profile.sh --kind <dispatch_kind>` fails closed (exit 3, empty
# stdout) when the resolved profile declares `roles:` that omit the kind, so the
# caller keeps its cloud tier. A profile with NO `roles:` key is unconstrained and
# routes byte-identically to today. TDD: exercises the real script, no mocks.

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/select-model-profile-roles"

setup() {
    mkdir -p "$FIXTURES_DIR"
    PROFILES_FILE="$FIXTURES_DIR/model-profiles.yml"
    export AUTOSPEC_MODEL_PROFILES="$PROFILES_FILE"
}

teardown() {
    rm -rf "$FIXTURES_DIR"
    unset AUTOSPEC_MODEL_PROFILES
}

# ── Role match: the kind is in the resolved profile's roles: ───────────────────

@test "roles: [implementer] + --kind implementer resolves (exit 0)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --kind implementer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

@test "flow list with several roles: the present kind passes" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer, lgtm-reviewer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

# ── Negative path: the kind is absent from the resolved profile's roles: ──────

@test "roles: [implementer] + --kind lgtm-reviewer fails closed (exit 3, empty)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "role guard also fails closed on --print-model (exit 3, empty)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --print-model --kind lgtm-reviewer
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "role guard also fails closed on --print-effort (exit 3, empty)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    effort: high
    roles: [implementer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --print-effort --kind lgtm-reviewer
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "block-style roles: list is honoured (absent kind fails closed)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles:
      - implementer
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "block-style roles: list is honoured (present kind passes)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles:
      - implementer
      - lgtm-reviewer
EOF
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

# ── Absent roles: is unconstrained — today's routing is byte-identical ────────

@test "no roles: key + --kind routes byte-identically (exit 0)" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
  claude-haiku-cloud:
    model: claude-haiku-4-5
    ctx: 64k
    reasoning: medium
EOF
    # deep -> sonnet regardless of kind; kind is inert with no roles: key.
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
    # shallow/medium -> haiku, also unconstrained.
    run bash "$SCRIPT" --labels "reasoning:shallow" --kind implementer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-cloud" ]
}

# ── Role scoping: only the RESOLVED profile's roles are checked ───────────────

@test "an adjacent profile's roles: never leak into the resolved one" {
    # haiku (the resolved profile for shallow) has NO roles; sonnet does.
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer]
  claude-haiku-cloud:
    model: claude-haiku-4-5
    ctx: 64k
    reasoning: medium
EOF
    # shallow resolves to haiku (no roles) -> unconstrained -> passes.
    run bash "$SCRIPT" --labels "reasoning:shallow" --kind lgtm-reviewer
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-cloud" ]
    # deep resolves to sonnet (roles: [implementer]) -> mismatch -> fail closed.
    run bash "$SCRIPT" --labels "reasoning:deep" --kind lgtm-reviewer
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

# ── --kind surface: help documents it, and no-kind behaviour is unchanged ─────

@test "--help mentions --kind and exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--kind"* ]]
}

@test "omitting --kind leaves role-guarded profiles fully routable" {
    cat > "$PROFILES_FILE" <<'EOF'
default: claude-sonnet-cloud
profiles:
  claude-sonnet-cloud:
    model: claude-sonnet-4-6
    ctx: 120k
    reasoning: deep
    roles: [implementer]
EOF
    run bash "$SCRIPT" --labels "reasoning:deep"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-cloud" ]
}

# ── Auto-init writer emits roles: [implementer] for local profiles ────────────
# The lock-step rule keeps the body identical across the three skill files, so a
# local profile that is role-guarded in one must be in all three.

@test "SKILL.md auto-init sample local profile declares roles: [implementer]" {
    skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    [ -f "$skill_md" ]
    grep -q "roles: \[implementer\]" "$skill_md"
}

@test "codex/prompt.md auto-init sample local profile declares roles: [implementer]" {
    codex_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
    [ -f "$codex_md" ]
    grep -q "roles: \[implementer\]" "$codex_md"
}

@test "opencode/agent.md auto-init sample local profile declares roles: [implementer]" {
    opencode_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
    [ -f "$opencode_md" ]
    grep -q "roles: \[implementer\]" "$opencode_md"
}
