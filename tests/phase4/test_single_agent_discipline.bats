#!/usr/bin/env bats
#
# Phase 4 single-agent absorbed-discipline lockstep test (issue #648).
#
# Asserts that all 3 autospec-run adapter files contain the canonical sentence
# stating that subagents spawned by background `Agent` calls do NOT inherit the
# `Agent` tool, plus a reference to the single-agent rewrite. Also asserts
# that the legacy `process(ISSUE) # foreground subagent` pattern is gone.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
TRIO=(
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
)

@test "trio contains the canonical single-agent constraint sentence" {
    for f in "${TRIO[@]}"; do
        run grep -F "Subagents spawned by background \`Agent\` calls do NOT inherit the \`Agent\` tool" "$f"
        [ "$status" -eq 0 ] || {
            echo "missing canonical constraint sentence in: $f"
            return 1
        }
    done
}

@test "trio references the single-agent absorbed-discipline rewrite" {
    for f in "${TRIO[@]}"; do
        run grep -F "single-agent absorbed-discipline" "$f"
        [ "$status" -eq 0 ] || {
            echo "missing 'single-agent absorbed-discipline' reference in: $f"
            return 1
        }
    done
}

@test "trio mentions top-level Agent launch requirement" {
    for f in "${TRIO[@]}"; do
        run grep -F "top-level agents launched directly by the main session orchestrator" "$f"
        [ "$status" -eq 0 ] || {
            echo "missing top-level orchestrator launch sentence in: $f"
            return 1
        }
    done
}

@test "legacy 'foreground subagent' Phase 4 pattern is gone" {
    for f in "${TRIO[@]}"; do
        # The legacy comment annotated the nested-dispatch dispatch line.
        # Allow no occurrences of the exact pattern that misled operators.
        if grep -F "process(ISSUE)   # foreground subagent" "$f" >/dev/null 2>&1; then
            echo "legacy 'process(ISSUE)   # foreground subagent' pattern still present in: $f"
            return 1
        fi
        if grep -F "dispatches a **foreground subagent**" "$f" >/dev/null 2>&1; then
            echo "legacy 'dispatches a **foreground subagent**' pattern still present in: $f"
            return 1
        fi
    done
}

@test "phase4-implementer.md states the single-agent assumption" {
    f="$REPO_ROOT/skills/autospec-run/prompts/phase4-implementer.md"
    run grep -F "single-agent" "$f"
    [ "$status" -eq 0 ]
    run grep -F "Subagents spawned by background \`Agent\` calls do NOT inherit the \`Agent\` tool" "$f"
    [ "$status" -eq 0 ]
}
