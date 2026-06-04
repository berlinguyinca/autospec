#!/usr/bin/env bats
# skills/autospec-run/tests/reviewer-tier.bats — named-content coverage for
# tier right-sizing (D4, issue #941): the fused guardian+LGTM reviewer runs
# Tier B for ALL issues, `AUTOSPEC_REVIEWER_TIER=opus` is the documented
# escape hatch back to Tier A, and the second Tier-A regression meta-review
# dispatch is folded into the single reviewer brief (no longer a second pass).

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"

RUN_TRIO=(
  "$REPO_ROOT/skills/autospec-run/SKILL.md"
  "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
  "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
)

AUTOSPEC_TRIO=(
  "$REPO_ROOT/skills/autospec/SKILL.md"
  "$REPO_ROOT/skills/autospec/codex/prompt.md"
  "$REPO_ROOT/skills/autospec/opencode/agent.md"
)

# ── Reviewer runs Tier B for all issues; env hatch restores Tier A ───────────

@test "run trio: reviewer Model tier is TIER_B for ALL issues (no per-label TIER_A split)" {
  for f in "${RUN_TRIO[@]}"; do
    # The reviewer Model tier line must declare TIER_B for all issues.
    grep -q '`TIER_B` for ALL issues' "$f" \
      || { echo "missing 'TIER_B for ALL issues' in $f"; return 1; }
    # The old per-label split ("TIER_A for regression/priority:high") must be gone
    # from the reviewer Model tier directive.
    ! grep -q '`TIER_A` for `regression`/`priority:high` issues' "$f" \
      || { echo "stale per-label TIER_A reviewer split still present in $f"; return 1; }
  done
}

@test "run trio: AUTOSPEC_REVIEWER_TIER env hatch documented (unset->sonnet, opus->Tier A)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'AUTOSPEC_REVIEWER_TIER' "$f" \
      || { echo "missing AUTOSPEC_REVIEWER_TIER in $f"; return 1; }
    grep -q 'opus' "$f" || { echo "missing opus hatch value in $f"; return 1; }
  done
}

@test "AGENTS.md: AUTOSPEC_REVIEWER_TIER env hatch documented" {
  grep -q 'AUTOSPEC_REVIEWER_TIER' "$REPO_ROOT/AGENTS.md"
}

# ── Regression meta-review folded into the single reviewer brief ─────────────

@test "run trio: second Tier-A regression meta-review dispatch removed" {
  for f in "${RUN_TRIO[@]}"; do
    ! grep -q 'dispatch a second `TIER_A` subagent' "$f" \
      || { echo "second TIER_A meta-review dispatch still present in $f"; return 1; }
  done
}

@test "run trio: regression gap-check folded into the single reviewer brief (reviewer-lessons preserved)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'reviewer-lessons.md' "$f" \
      || { echo "reviewer-lessons write-path lost in $f"; return 1; }
    grep -q 'would the reviewer have caught the original gap' "$f" \
      || { echo "folded original-gap check missing in $f"; return 1; }
  done
}

# ── The autospec umbrella skill mirrors the same reviewer block ──────────────

@test "autospec umbrella trio: second Tier-A regression meta-review dispatch removed" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    ! grep -q 'dispatch a second `TIER_A` subagent' "$f" \
      || { echo "second TIER_A meta-review dispatch still present in $f"; return 1; }
  done
}
