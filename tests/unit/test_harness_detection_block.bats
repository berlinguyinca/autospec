#!/usr/bin/env bats
# tests/unit/test_harness_detection_block.bats — exercise check_harness_detection_block()
# and the updated check_subagent_model_tier() in scripts/validate.sh.
#
# Four cases per issue #293:
#   1. Old verbose tier brief format passes check_subagent_model_tier
#   2. New TIER_A/TIER_B format passes check_subagent_model_tier
#   3. Missing ## Harness detection section emits WARN not FAIL
#   4. Present but incomplete harness detection section (missing TIER_B) emits FAIL

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    SCRATCH="$(mktemp -d)"
    export SCRATCH REPO_ROOT VALIDATE

    # Create a self-contained helper script that sources only the two functions
    # under test from validate.sh, so we can call them in isolation.
    HELPER="$SCRATCH/helper.sh"
    cat > "$HELPER" <<'HELPER_SCRIPT'
#!/usr/bin/env bash
set -eu

fail() { printf 'validate: FAIL — %s\n' "$*" >&2; exit 1; }
info() { printf 'validate: %s\n' "$*"; }

HELPER_SCRIPT

    # Extract check_harness_detection_block from validate.sh and append it
    sed -n '/^check_harness_detection_block()/,/^}/p' "$VALIDATE" >> "$HELPER"

    # Extract check_subagent_model_tier from validate.sh and append it
    sed -n '/^check_subagent_model_tier()/,/^}/p' "$VALIDATE" >> "$HELPER"

    export HELPER
}

teardown() {
    if [ -n "${SCRATCH:-}" ] && [ -d "$SCRATCH" ]; then
        rm -rf "$SCRATCH"
    fi
}

# ===========================================================================
# Test 1: Old verbose tier brief format passes check_subagent_model_tier
# ===========================================================================
@test "check_subagent_model_tier: old verbose format (Tier A/Tier B) passes" {
    # The real repo currently uses the old verbose format — validate.sh must exit 0.
    # This confirms check_subagent_model_tier still accepts legacy format.
    run bash "$VALIDATE"
    [ "$status" -eq 0 ]
    # Confirm at least one skill still uses the legacy phrase (sanity check)
    run grep -rl 'Tier A (spec work)' "$REPO_ROOT/skills/"
    [ "$status" -eq 0 ]
}

# ===========================================================================
# Test 2: New TIER_A/TIER_B format passes check_subagent_model_tier
# ===========================================================================
@test "check_subagent_model_tier: new TIER_A/TIER_B format passes" {
    # Build a minimal fake skill_dir with SKILL.md using TIER_A/TIER_B format.
    skill_dir="$SCRATCH/skills/fake-skill"
    mkdir -p "$skill_dir"
    cat > "$skill_dir/SKILL.md" <<'EOF'
---
name: fake-skill
version: 0.1.0
---

## Harness detection (run once at skill start, before Phase 0)

Detect your harness. TIER_A = opus. TIER_B = sonnet. Silently fall back to TIER_A if unavailable.

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.
> **Model tier:** `TIER_B` (implementation work) — cheaper model with medium thinking; resolved at startup. Silently fall back to `TIER_A` if unavailable.

| Subagent model tier | TIER_A / TIER_B |
| Other row | value |
EOF

    run bash -c "source '$HELPER' && check_subagent_model_tier '$skill_dir'" 2>&1
    [ "$status" -eq 0 ]
}

# ===========================================================================
# Test 3: Missing ## Harness detection section emits WARN not FAIL
# ===========================================================================
@test "check_harness_detection_block: missing section emits WARN not FAIL" {
    skill_file="$SCRATCH/no-harness.md"
    cat > "$skill_file" <<'EOF'
---
name: fake-skill
version: 0.1.0
---

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.

| Subagent model tier | TIER_A |
EOF

    # Must exit 0 (WARN, not FAIL)
    run bash -c "source '$HELPER' && check_harness_detection_block '$skill_file'" 2>&1
    [ "$status" -eq 0 ]
    # Must emit a WARN line mentioning Harness detection
    echo "$output" | grep -i 'WARN'
    echo "$output" | grep -i 'Harness detection'
}

# ===========================================================================
# Test 4: Present but incomplete harness detection section (missing TIER_B) → FAIL
# ===========================================================================
@test "check_harness_detection_block: section present but missing TIER_B emits FAIL" {
    skill_file="$SCRATCH/incomplete-harness.md"
    cat > "$skill_file" <<'EOF'
---
name: fake-skill
version: 0.1.0
---

## Harness detection (run once at skill start, before Phase 0)

Detect your harness. TIER_A = opus (top tier). Silently fall back if the top tier is unavailable.
The second-tier reference is absent from this fixture on purpose.

EOF

    # Must fail (non-zero exit) — TIER_B is absent so the check must fail
    run bash -c "source '$HELPER' && check_harness_detection_block '$skill_file'" 2>&1
    [ "$status" -ne 0 ]
    # Must mention TIER_B in the error message
    echo "$output" | grep -i 'TIER_B'
}

# ===========================================================================
# Bonus: validate.sh defines check_harness_detection_block
# ===========================================================================
@test "validate.sh: defines check_harness_detection_block" {
    run grep -q '^check_harness_detection_block()' "$VALIDATE"
    [ "$status" -eq 0 ]
}

# ===========================================================================
# Bonus: validate.sh still passes against the real repo (regression guard)
# ===========================================================================
@test "validate.sh: passes against real repo with WARN for missing harness detection" {
    run bash "$VALIDATE"
    [ "$status" -eq 0 ]
    # WARNs should be present (skills don't yet have harness detection blocks)
    echo "$output" | grep -i 'WARN.*Harness detection'
}
