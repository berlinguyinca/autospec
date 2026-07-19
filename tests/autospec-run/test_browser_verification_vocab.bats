#!/usr/bin/env bats
# Locks the Phase 4 browser verification vocabulary into the autospec-run trio
# and the absorbed implementer prompt that actually gates PR validation.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    RUN_DIR="$REPO_ROOT/skills/autospec-run"
    PHASE4_PROMPT="$RUN_DIR/prompts/phase4-implementer.md"
}

@test "autospec-run trio references all browser verification states" {
    for file in "$RUN_DIR/SKILL.md" "$RUN_DIR/codex/prompt.md" "$RUN_DIR/opencode/agent.md"; do
        grep -q 'browser-verified' "$file"
        grep -q 'fallback-smoke-only' "$file"
        grep -q 'not-run' "$file"
    done
}

@test "phase4 implementer prompt requires remediation for harness-caused browser skips" {
    grep -q 'browser-verified' "$PHASE4_PROMPT"
    grep -q 'fallback-smoke-only' "$PHASE4_PROMPT"
    grep -q 'not-run' "$PHASE4_PROMPT"
    grep -qiE 'remediation issue.*browser|browser.*remediation issue' "$PHASE4_PROMPT"
    grep -qi 'harness' "$PHASE4_PROMPT"
}
