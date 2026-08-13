#!/usr/bin/env bash
# tests/phase4/test_worktree_ladder_assert.sh
# Verifies the PR-aware recovery ladder + mandatory worktree-guard.sh assert
# wiring (issue #961, spec §D3):
#   1. All 6 run-trio files carry the 3-state ladder (open-pr/branch-only/fresh)
#      wired through `worktree-guard.sh resolve-branch`.
#   2. All 6 run-trio files carry the mandatory `worktree-guard.sh assert` gate
#      that MUST exit 0 before any edit, stopping the issue on non-zero exit
#      (comment + restore auto-implement).
#   3. All 6 files carry the prose hard rules (no primary-checkout mutation;
#      cleanup = git worktree remove after merge + git worktree prune).
#   4. phase4-implementer.md carries the same assert + ladder instructions.
#   5. The ladder bash extracted from each SKILL.md is well-formed (bash -n).
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

fail() { echo "FAIL: $1" >&2; exit 1; }

# ── File sets for both run trios ─────────────────────────────────────────────

RUN_SKILL="$SCRIPT_DIR/skills/autospec-run/SKILL.md"
RUN_CODEX="$SCRIPT_DIR/skills/autospec-run/codex/prompt.md"
RUN_OPENCODE="$SCRIPT_DIR/skills/autospec-run/opencode/agent.md"

AS_SKILL="$SCRIPT_DIR/skills/autospec/SKILL.md"
AS_CODEX="$SCRIPT_DIR/skills/autospec/codex/prompt.md"
AS_OPENCODE="$SCRIPT_DIR/skills/autospec/opencode/agent.md"

ALL_SIX="$RUN_SKILL $RUN_CODEX $RUN_OPENCODE $AS_SKILL $AS_CODEX $AS_OPENCODE"

PHASE4="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"

# ── 1. Ladder: 3 states wired through resolve-branch, in all 6 trio files ─────

for f in $ALL_SIX $PHASE4; do
    name="${f#"$SCRIPT_DIR"/}"

    grep -q 'worktree-guard.sh resolve-branch' "$f" \
        || fail "$name: missing worktree-guard.sh resolve-branch (ladder step 1)"

    # All three ladder states must be named.
    grep -q 'open-pr' "$f" \
        || fail "$name: missing open-pr ladder state"
    grep -q 'branch-only' "$f" \
        || fail "$name: missing branch-only ladder state"
    grep -q '\bfresh\b' "$f" \
        || fail "$name: missing fresh ladder state"

    # open-pr semantics: skip implementation, verify+review+merge the existing PR.
    grep -qi 'skip implementation' "$f" \
        || fail "$name: open-pr path must skip implementation (#886)"

    # branch-only semantics: adopt and continue.
    grep -q 'adopt' "$f" \
        || fail "$name: branch-only path must adopt the branch (#917)"
done

# ── 2. Mandatory assert gate before any edit, stop-on-failure, in all 6 ──────

for f in $RUN_SKILL $RUN_CODEX $RUN_OPENCODE $PHASE4; do
    name="${f#"$SCRIPT_DIR"/}"

    grep -q "worktree-guard.sh assert --expected-branch <BRANCH> --branch-pattern 'feat/\*'" "$f" \
        || fail "$name: missing exact feat/* branch identity gate (step 2)"

done

for f in $ALL_SIX $PHASE4; do
    name="${f#"$SCRIPT_DIR"/}"

    grep -q 'MUST exit 0' "$f" \
        || fail "$name: assert gate must require exit 0 before any edit"

    # Non-zero assert stops the issue: comment + restore auto-implement.
    grep -q 'restore .*auto-implement\|restore `auto-implement`\|swap .*auto-implement\|`in-progress-by-bot` . `auto-implement`' "$f" \
        || fail "$name: non-zero assert must restore auto-implement and stop"
done

# ── 3. Prose hard rules: no primary-checkout mutation; cleanup + prune ────────

for f in $ALL_SIX $PHASE4; do
    name="${f#"$SCRIPT_DIR"/}"

    grep -qi 'primary checkout' "$f" \
        || fail "$name: missing primary-checkout hard rule"
    grep -q 'git worktree remove' "$f" \
        || fail "$name: missing 'git worktree remove' cleanup rule"
    grep -q 'git worktree prune' "$f" \
        || fail "$name: missing 'git worktree prune' cleanup rule"
done

# ── 4. Ladder bash extracted from each SKILL.md is well-formed ───────────────
# Extract the LADDER fenced block (delimited by sentinel comments) and bash -n it.

for f in "$RUN_SKILL" "$AS_SKILL"; do
    name="${f#"$SCRIPT_DIR"/}"
    block="$(awk '
        /worktree-ladder:begin/{cap=1; next}
        /worktree-ladder:end/{cap=0}
        cap{ sub(/^> ?/, ""); print }
    ' "$f")"
    [ -n "$block" ] \
        || fail "$name: missing worktree-ladder:begin/end sentinel block"
    printf '%s\n' "$block" | bash -n - \
        || fail "$name: extracted worktree-ladder block fails bash -n"
done

echo "ok: worktree ladder + assert wiring present in all 6 trio files + phase4-implementer.md"
