#!/usr/bin/env bats
# skills/autospec-test/tests/unit/validate-skill-structure.bats
#
# TDD tests for Phase 10 deliverables:
#   - skills/autospec-test/validate.sh — structural lint
#   - skills/autospec-test/SKILL.md — required sections present
#   - skills/autospec-test/codex/prompt.md — leading blank line
#
# Run: bats skills/autospec-test/tests/unit/validate-skill-structure.bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec-test"
    VALIDATE_SH="$SKILL_DIR/validate.sh"
    SKILL_MD="$SKILL_DIR/SKILL.md"
    CODEX_PROMPT="$SKILL_DIR/codex/prompt.md"
    TMPDIR_TEST="$(mktemp -d /tmp/autospec-validate-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TMPDIR_TEST"
}

# ── validate.sh exists and passes bash -n ─────────────────────────────────────

@test "validate.sh exists" {
    [ -f "$VALIDATE_SH" ]
}

@test "validate.sh passes bash syntax check" {
    bash -n "$VALIDATE_SH"
}

# ── SKILL.md structural sections ──────────────────────────────────────────────

@test "SKILL.md exists" {
    [ -f "$SKILL_MD" ]
}

@test "SKILL.md contains ## Self-update section" {
    grep -q '^## Self-update' "$SKILL_MD"
}

@test "SKILL.md contains ## Model tier section" {
    grep -q '^## Model tier' "$SKILL_MD"
}

@test "SKILL.md Model tier declares reasoning:standard" {
    grep -q 'reasoning:standard' "$SKILL_MD"
}

@test "SKILL.md Model tier declares ctx:120k" {
    grep -q 'ctx:120k' "$SKILL_MD"
}

@test "SKILL.md contains ## When to use section" {
    grep -q '^## When to use' "$SKILL_MD"
}

@test "SKILL.md contains ## When not to use section" {
    grep -q '^## When not to use' "$SKILL_MD"
}

@test "SKILL.md contains ## How it works section" {
    grep -q '^## How it works' "$SKILL_MD"
}

@test "SKILL.md contains ## Contract file section" {
    grep -q '^## Contract file' "$SKILL_MD"
}

@test "SKILL.md contains ## Modes I and II section" {
    grep -q '^## Modes I and II' "$SKILL_MD"
}

@test "SKILL.md contains ## Safety rails section" {
    grep -q '^## Safety rails' "$SKILL_MD"
}

@test "SKILL.md contains ## Self-heal loop section" {
    grep -q '^## Self-heal loop' "$SKILL_MD"
}

@test "SKILL.md contains ## Wizard section" {
    grep -q '^## Wizard' "$SKILL_MD"
}

@test "SKILL.md contains ## Stop mode section (pure prose)" {
    grep -q '^## Stop mode' "$SKILL_MD"
}

@test "SKILL.md Stop mode section has no FEATURE_DESCRIPTION heredoc" {
    # Pure prose: no shell-out of user text (saved-memory lockstep rule 4)
    # Check for the actual forbidden placeholder form: {FEATURE_DESCRIPTION}
    run grep '{FEATURE_DESCRIPTION}' "$SKILL_MD"
    [ "$status" -ne 0 ]
}

@test "SKILL.md contains forbidden_url_patterns example block (adapter row)" {
    grep -q 'forbidden_url_patterns' "$SKILL_MD"
}

@test "SKILL.md has YAML frontmatter with name field" {
    # Check frontmatter exists with name: autospec-test
    head -5 "$SKILL_MD" | grep -q '^---'
    awk '/^---$/{c++; next} c==1{print}' "$SKILL_MD" | grep -q '^name:'
}

# ── codex/prompt.md leading blank line ───────────────────────────────────────

@test "codex/prompt.md exists" {
    [ -f "$CODEX_PROMPT" ]
}

@test "codex/prompt.md starts with a leading blank line (byte-precise)" {
    # The very first byte of the file must be a newline (leading blank line convention)
    first_char="$(head -c 1 "$CODEX_PROMPT" | xxd -p)"
    [ "$first_char" = "0a" ]
}

@test "codex/prompt.md is not empty after blank line" {
    line_count="$(wc -l < "$CODEX_PROMPT" | tr -d ' ')"
    [ "$line_count" -gt 5 ]
}

# ── validate.sh rejects SKILL.md with missing sections ───────────────────────

@test "validate.sh rejects a SKILL.md missing ## Self-update section" {
    # Create a fake skill dir with a SKILL.md missing Self-update
    fake_dir="$TMPDIR_TEST/fake-skill"
    mkdir -p "$fake_dir/codex"
    # Write SKILL.md without ## Self-update
    cat > "$fake_dir/SKILL.md" <<'EOF'
---
name: fake-skill
description: fake
---

## Model tier

reasoning:standard, ctx:120k

## When to use

Something.

## When not to use

Something.

## How it works

Something.

## Contract file

Something.

## Modes I and II

Something.

## Safety rails

Something.

## Self-heal loop

Something.

## Wizard

Something.

## Stop mode

Pure prose stop mode.
EOF
    # Write a valid codex/prompt.md with leading blank line
    printf '\n# fake-skill\n\nContent.\n' > "$fake_dir/codex/prompt.md"
    # Run validate.sh against this dir — must exit non-zero
    run bash "$VALIDATE_SH" --skill-dir "$fake_dir"
    [ "$status" -ne 0 ]
}

@test "validate.sh rejects a SKILL.md missing ## Model tier section" {
    fake_dir="$TMPDIR_TEST/fake-skill-no-model-tier"
    mkdir -p "$fake_dir/codex"
    cat > "$fake_dir/SKILL.md" <<'EOF'
---
name: fake-skill
description: fake
---

## Self-update

Pure prose.

## When to use

Something.

## When not to use

Something.

## How it works

Something.

## Contract file

Something.

## Modes I and II

Something.

## Safety rails

Something.

## Self-heal loop

Something.

## Wizard

Something.

## Stop mode

Pure prose stop mode.
EOF
    printf '\n# fake-skill\n\nContent.\n' > "$fake_dir/codex/prompt.md"
    run bash "$VALIDATE_SH" --skill-dir "$fake_dir"
    [ "$status" -ne 0 ]
}

@test "validate.sh rejects codex/prompt.md without leading blank line" {
    fake_dir="$TMPDIR_TEST/fake-skill-no-blank"
    mkdir -p "$fake_dir/codex"
    # Full valid SKILL.md
    cat > "$fake_dir/SKILL.md" <<'EOF'
---
name: fake-skill
description: fake
---

## Self-update

Pure prose.

## Model tier

reasoning:standard, ctx:120k

## When to use

Something.

## When not to use

Something.

## How it works

Something.

## Contract file

Something.

## Modes I and II

Something.

## Safety rails

Something.

## Self-heal loop

Something.

## Wizard

Something.

## Stop mode

Pure prose stop mode.
EOF
    # codex/prompt.md WITHOUT leading blank line
    printf '# fake-skill\n\nContent.\n' > "$fake_dir/codex/prompt.md"
    run bash "$VALIDATE_SH" --skill-dir "$fake_dir"
    [ "$status" -ne 0 ]
}

@test "validate.sh accepts valid SKILL.md and codex/prompt.md" {
    fake_dir="$TMPDIR_TEST/fake-skill-valid"
    mkdir -p "$fake_dir/codex"
    cat > "$fake_dir/SKILL.md" <<'EOF'
---
name: fake-skill
description: fake
---

## Self-update

Pure prose self-update section.

## Model tier

reasoning:standard, ctx:120k

## When to use

Something.

## When not to use

Something.

## How it works

Something.

## Contract file

forbidden_url_patterns example block here.

## Modes I and II

Something.

## Safety rails

Something.

## Self-heal loop

Something.

## Wizard

Something.

## Stop mode

Pure prose stop mode. No shell-out placeholders.
EOF
    printf '\n# fake-skill\n\nContent.\n' > "$fake_dir/codex/prompt.md"
    run bash "$VALIDATE_SH" --skill-dir "$fake_dir"
    [ "$status" -eq 0 ]
}
