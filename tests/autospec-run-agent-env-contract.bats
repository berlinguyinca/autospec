#!/usr/bin/env bats
# tests/autospec-run-agent-env-contract.bats — runtime isolation contract in autospec-run surfaces

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "autospec-run provisions the Rust runtime broker after worktree assertion" {
  for surface in \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
    grep -q '<!-- autospec-block:runtime-resource-preflight -->' "$surface"
    expanded="$BATS_TEST_TMPDIR/legacy-$(basename "$(dirname "$surface")")-$(basename "$surface")"
    bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$surface" > "$expanded"
    grep -q 'autospec runtime env up --repo "$PWD"' "$expanded"
    grep -q 'AUTOSPEC_PUBLIC_URL.*canonical browser/QA URL' "$expanded"
    grep -q 'autospec runtime env down --repo /tmp/wt-<BRANCH>' "$surface"
    ! grep -q 'agent-env.sh' "$surface"
  done
}

@test "autospec and autospec-run always normalize Compose before broker up" {
  for surface in \
    "$REPO_ROOT/skills/autospec/SKILL.md" \
    "$REPO_ROOT/skills/autospec/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
    grep -q '<!-- autospec-block:runtime-resource-preflight -->' "$surface"
    expanded="$BATS_TEST_TMPDIR/$(basename "$(dirname "$surface")")-$(basename "$surface")"
    bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$surface" > "$expanded"
    grep -q 'autospec runtime env normalize-compose --repo "$PWD" --check' "$expanded"
    normalize_line="$(grep -n -m1 'normalize-compose --repo' "$expanded" | cut -d: -f1)"
    up_line="$(grep -n -m1 'autospec runtime env up --repo' "$expanded" | cut -d: -f1)"
    [ -n "$normalize_line" ]
    [ -n "$up_line" ]
    [ "$normalize_line" -lt "$up_line" ]
  done
}

@test "runtime preflight is unconditional even when no manifest exists" {
  block="$REPO_ROOT/templates/skill-blocks/runtime-resource-preflight.md"
  [ -f "$block" ]
  grep -q 'autospec runtime env normalize-compose --repo "$PWD" --check' "$block"
  ! grep -B2 -A2 'normalize-compose --repo' "$block" | \
    grep -Eq '\[ -f .*runtime\.yml|manifest.*exists'
}

@test "Phase 4 cleanup releases runtime resources before Git removal" {
  for surface in \
    "$REPO_ROOT/skills/autospec/SKILL.md" \
    "$REPO_ROOT/skills/autospec/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
    grep -E 'autospec runtime env down --repo /tmp/wt-<BRANCH>.*autospec-runtime-worktree-cleanup\.sh.*worktree remove /tmp/wt-<BRANCH>' "$surface"
    ! grep -E 'autospec runtime env down .*\|\| true' "$surface"
  done
}
