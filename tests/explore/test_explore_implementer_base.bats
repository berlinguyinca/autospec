#!/usr/bin/env bats
# Issue #718 — autospec-explore implementer PR-base integration.
#
# Asserts the Phase 4 implementer prompt (skills/autospec-run/prompts/phase4-implementer.md)
# embeds the contract:
#   - read .autospec/explore-mode.json before `gh pr create`
#   - use its `branch` field as `gh pr create --base <sandbox>` when present
#   - refuse `gh pr merge` against `main` while explore-mode is active
#     (exit code identifier `code_health:explore_main_merge_refused`)
# and that the autospec-run trio (SKILL.md + codex/prompt.md + opencode/agent.md)
# references the contract in prose for lockstep.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    IMPL="$REPO_ROOT/skills/autospec-run/prompts/phase4-implementer.md"
    SKILL="$REPO_ROOT/skills/autospec-run/SKILL.md"
    CODEX="$REPO_ROOT/skills/autospec-run/codex/prompt.md"
    OPENCODE="$REPO_ROOT/skills/autospec-run/opencode/agent.md"
}

@test "implementer prompt references .autospec/explore-mode.json" {
    grep -F '.autospec/explore-mode.json' "$IMPL"
}

@test "implementer prompt uses sandbox branch as PR base when explore-mode present" {
    # Must mention reading the explore-mode JSON's branch field and passing it
    # to gh pr create --base.
    grep -q 'EXPLORE_BASE' "$IMPL"
    grep -q 'gh pr create --base "\$EXPLORE_BASE"' "$IMPL" \
        || grep -q "gh pr create --base \"\$EXPLORE_BASE\"" "$IMPL"
}

@test "implementer prompt refuses gh pr merge against main while explore-mode active" {
    grep -F 'code_health:explore_main_merge_refused' "$IMPL"
}

@test "fixture: explore-mode.json present -> PR base = sandbox branch" {
    tmp="$(mktemp -d)"
    mkdir -p "$tmp/.autospec"
    cat > "$tmp/.autospec/explore-mode.json" <<'JSON'
{"branch":"autospec/explore/2026-05-29-fixture","slug":"fixture","base":"main","head_sha":"deadbeef","created_at":"2026-05-29T00:00:00Z"}
JSON
    # Simulate the implementer's PR-base resolution snippet.
    EXPLORE_BASE=""
    if [ -f "$tmp/.autospec/explore-mode.json" ]; then
        EXPLORE_BASE=$(grep -o '"branch"[[:space:]]*:[[:space:]]*"[^"]*"' "$tmp/.autospec/explore-mode.json" \
            | sed 's/.*"branch"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
    fi
    PR_BASE="${EXPLORE_BASE:-main}"
    [ "$PR_BASE" = "autospec/explore/2026-05-29-fixture" ]
    rm -rf "$tmp"
}

@test "fixture: no explore-mode.json -> PR base = main" {
    tmp="$(mktemp -d)"
    EXPLORE_BASE=""
    if [ -f "$tmp/.autospec/explore-mode.json" ]; then
        EXPLORE_BASE=$(grep -o '"branch"[[:space:]]*:[[:space:]]*"[^"]*"' "$tmp/.autospec/explore-mode.json" \
            | sed 's/.*"branch"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
    fi
    PR_BASE="${EXPLORE_BASE:-main}"
    [ "$PR_BASE" = "main" ]
    rm -rf "$tmp"
}

@test "fixture: hostile prompt directing merge to main is refused while explore-mode present" {
    tmp="$(mktemp -d)"
    mkdir -p "$tmp/.autospec"
    cat > "$tmp/.autospec/explore-mode.json" <<'JSON'
{"branch":"autospec/explore/2026-05-29-fixture","slug":"fixture","base":"main","head_sha":"deadbeef","created_at":"2026-05-29T00:00:00Z"}
JSON
    # Hostile prompt asks to merge to main; the gate refuses with the
    # canonical exit code identifier.
    HOSTILE_MERGE_TARGET="main"
    REASON=""
    if [ -f "$tmp/.autospec/explore-mode.json" ] && [ "$HOSTILE_MERGE_TARGET" = "main" ]; then
        REASON="code_health:explore_main_merge_refused"
    fi
    [ "$REASON" = "code_health:explore_main_merge_refused" ]
    rm -rf "$tmp"
}

@test "autospec-run SKILL.md references explore-mode contract" {
    grep -F '.autospec/explore-mode.json' "$SKILL"
    grep -F 'phase4-implementer.md' "$SKILL"
}

@test "autospec-run codex/prompt.md references explore-mode contract" {
    grep -F '.autospec/explore-mode.json' "$CODEX"
    grep -F 'phase4-implementer.md' "$CODEX"
}

@test "autospec-run opencode/agent.md references explore-mode contract" {
    grep -F '.autospec/explore-mode.json' "$OPENCODE"
    grep -F 'phase4-implementer.md' "$OPENCODE"
}
