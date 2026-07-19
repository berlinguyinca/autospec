#!/usr/bin/env bats
# tests/unit/test_autospec_doc_sweep_define_wiring.bats — issue #924 / spec §D6.
#
# Asserts the two-trio wiring of the single doc engine:
#   * autospec-sweep docs-drift check invokes `/autospec-doc --full`
#     (full regen + repo-wide completeness audit via doc-orchestrator.mjs).
#   * autospec-define Auto-docs step invokes `/autospec-doc`
#     (doc-orchestrator.mjs) and the old parallel gen-docs-from-spec.mjs
#     generation path is gone from that step (one engine only).
#   * Both trios remain byte-identical below adapter headers (lock-step).

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "sweep docs-drift adapter invokes /autospec-doc --full via doc-orchestrator" {
  # The adapter resolves the orchestrator path then runs it with --full.
  grep -q 'doc-orchestrator.mjs' "$REPO_ROOT/scripts/dogfood-adapter-doc-drift.sh"
  grep -qE 'node "\$ORCHESTRATOR" --full' "$REPO_ROOT/scripts/dogfood-adapter-doc-drift.sh"
}

@test "sweep trio names /autospec-doc --full for the docs-drift area" {
  for f in \
    "$REPO_ROOT/skills/autospec-sweep/SKILL.md" \
    "$REPO_ROOT/skills/autospec-sweep/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-sweep/codex/prompt.md"
  do
    grep -q -- '/autospec-doc --full' "$f"
  done
}

@test "define Auto-docs step invokes /autospec-doc via doc-orchestrator across the trio" {
  for f in \
    "$REPO_ROOT/skills/autospec-define/SKILL.md" \
    "$REPO_ROOT/skills/autospec-define/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec-define/codex/prompt.md"
  do
    # The Auto-docs step block (after "### Auto-docs step") references the
    # orchestrator, not the old parallel generator.
    block="$(awk '/^### Auto-docs step/{f=1} f{print} /^## Autonomous mode/{if(f)exit}' "$f")"
    printf '%s' "$block" | grep -q 'doc-orchestrator.mjs'
    ! printf '%s' "$block" | grep -q 'gen-docs-from-spec.mjs'
  done
}

@test "both trios pass validate.sh lock-step (byte-identical below adapter headers)" {
  run bash "$REPO_ROOT/autospec validate"
  [ "$status" -eq 0 ]
}
