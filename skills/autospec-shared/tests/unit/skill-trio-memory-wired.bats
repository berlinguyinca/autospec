#!/usr/bin/env bats
# skill-trio-memory-wired.bats — assert auto-init-memory.sh is wired into every
# autospec-* SKILL trio (SKILL.md + codex/prompt.md + opencode/agent.md).
#
# Run: bats skills/autospec-shared/tests/unit/skill-trio-memory-wired.bats
# (from repo root)

# tests/unit is 2 levels deep inside autospec-shared, which is inside skills/
# So go up 4 levels: unit -> tests -> autospec-shared -> skills -> repo root
REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../../.." && pwd)"

# Skills that have the startup self-update block and therefore require wiring.
# autospec-e2e-clone, autospec-review, and autospec-test have no self-update
# block and are excluded per issue #500 scope.
WIRED_SKILLS=(
  autospec
  autospec-classify
  autospec-define
  autospec-listen
  autospec-run
  autospec-split
  autospec-stop
  autospec-story
)

# ── SKILL.md checks ──────────────────────────────────────────────────────────

@test "every wired skill SKILL.md contains exactly one auto-init-memory.sh invocation" {
  for skill in "${WIRED_SKILLS[@]}"; do
    local f="$REPO_ROOT/skills/$skill/SKILL.md"
    [ -f "$f" ] || { echo "MISSING: $f" >&2; return 1; }
    local count
    count=$(bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$f" 2>/dev/null | grep -cF 'auto-init-memory.sh' || true)
    [ "$count" -eq 1 ] || { echo "$skill/SKILL.md: expected 1 occurrence, got $count" >&2; return 1; }
  done
}

# ── codex/prompt.md checks ───────────────────────────────────────────────────

@test "every wired skill codex/prompt.md contains exactly one auto-init-memory.sh invocation" {
  for skill in "${WIRED_SKILLS[@]}"; do
    local f="$REPO_ROOT/skills/$skill/codex/prompt.md"
    [ -f "$f" ] || { echo "MISSING: $f" >&2; return 1; }
    local count
    count=$(bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$f" 2>/dev/null | grep -cF 'auto-init-memory.sh' || true)
    [ "$count" -eq 1 ] || { echo "$skill/codex/prompt.md: expected 1 occurrence, got $count" >&2; return 1; }
  done
}

# ── opencode/agent.md checks (trio skills only) ──────────────────────────────

@test "every wired trio skill opencode/agent.md contains exactly one auto-init-memory.sh invocation" {
  for skill in "${WIRED_SKILLS[@]}"; do
    local f="$REPO_ROOT/skills/$skill/opencode/agent.md"
    [ -f "$f" ] || { echo "MISSING (expected trio): $f" >&2; return 1; }
    local count
    count=$(bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$f" 2>/dev/null | grep -cF 'auto-init-memory.sh' || true)
    [ "$count" -eq 1 ] || { echo "$skill/opencode/agent.md: expected 1 occurrence, got $count" >&2; return 1; }
  done
}

# ── Byte-identical two-line block ────────────────────────────────────────────

@test "auto-init block is byte-identical across each skill trio" {
  for skill in "${WIRED_SKILLS[@]}"; do
    local skill_md="$REPO_ROOT/skills/$skill/SKILL.md"
    local codex_md="$REPO_ROOT/skills/$skill/codex/prompt.md"
    local opencode_md="$REPO_ROOT/skills/$skill/opencode/agent.md"

    local block_skill block_codex block_opencode
    local exp="$REPO_ROOT/scripts/expand-skill-blocks.sh"
    block_skill=$(bash "$exp" "$skill_md" 2>/dev/null | grep -A1 '# Auto-init cross-tool memory' || true)
    block_codex=$(bash "$exp" "$codex_md" 2>/dev/null | grep -A1 '# Auto-init cross-tool memory' || true)
    block_opencode=$(bash "$exp" "$opencode_md" 2>/dev/null | grep -A1 '# Auto-init cross-tool memory' || true)

    [ -n "$block_skill" ] || { echo "$skill/SKILL.md: auto-init comment block not found" >&2; return 1; }
    [ "$block_skill" = "$block_codex" ] \
      || { echo "$skill: SKILL.md vs codex/prompt.md auto-init block differs" >&2; return 1; }
    [ "$block_skill" = "$block_opencode" ] \
      || { echo "$skill: SKILL.md vs opencode/agent.md auto-init block differs" >&2; return 1; }
  done
}

# ── validate.sh passes ───────────────────────────────────────────────────────
#
# QUARANTINE (epic #1280 bats hang sweep): this test shells out to the FULL
# `bash scripts/validate.sh`, which runs the entire ~13-minute gate (including
# the context-monitor python suites). When this unit file is picked up by a
# directory-sweeping runner (`bats -r`, CI "run all bats", or this repo's own
# bats sweep), that 13-minute call looks like a hang and freezes the runner —
# exactly the "a test holds the pipe and freezes CI" failure mode this sweep
# is chartered to eliminate. The check is also redundant: validate.sh already
# has its own dedicated bats coverage and is invoked directly by CI, so re-
# running it from inside an unrelated "memory-wired" unit suite buys nothing.
#
# Gated behind AUTOSPEC_RUN_SLOW_VALIDATE_IN_BATS=1 so it can still be invoked
# deliberately, but never freezes an unattended sweep. Skipped by default.
@test "bash scripts/validate.sh exits 0" {
  [ "${AUTOSPEC_RUN_SLOW_VALIDATE_IN_BATS:-0}" = "1" ] \
    || skip "quarantined: runs full ~13min validate.sh (set AUTOSPEC_RUN_SLOW_VALIDATE_IN_BATS=1 to enable); validate.sh is gated directly by CI"
  run bash "$REPO_ROOT/scripts/validate.sh"
  if [ "$status" -ne 0 ]; then
    echo "validate.sh output:" >&2
    echo "$output" >&2
  fi
  [ "$status" -eq 0 ]
}
