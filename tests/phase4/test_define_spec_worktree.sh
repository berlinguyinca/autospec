#!/usr/bin/env bash
# tests/phase4/test_define_spec_worktree.sh
# Named-content checks for the autospec-define spec-PR worktree routing (issue
# #962, spec §D4) and the AGENTS.md ## Git hygiene (agents) section (§D5).
#
# Verified invariants:
#   1. All 3 define-trio files carry the define-spec-pr sentinel block.
#   2. All 3 files reference worktree-guard.sh and /tmp/wt-spec-<slug>.
#   3. All 3 files call `worktree-guard.sh assert` before spec commit.
#   4. All 3 files call `resolve-branch` (PR-aware ladder).
#   5. All 3 files carry cleanup (git worktree remove + git worktree prune).
#   6. AGENTS.md carries ## Git hygiene (agents) section with all 5 sub-rules.
#   7. The define-spec-pr sentinel block bash is well-formed (bash -n).
#   8. Lock-step: SKILL.md body == codex/prompt.md and == opencode/agent.md body.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

fail() { echo "FAIL: $1" >&2; exit 1; }
info() { echo "ok: $1"; }

SKILL="$SCRIPT_DIR/skills/autospec-define/SKILL.md"
CODEX="$SCRIPT_DIR/skills/autospec-define/codex/prompt.md"
OPENCODE="$SCRIPT_DIR/skills/autospec-define/opencode/agent.md"
AGENTS="$SCRIPT_DIR/AGENTS.md"
ALL_TRIO="$SKILL $CODEX $OPENCODE"

# ── 1-5. Define trio: spec-PR worktree routing ────────────────────────────────

for f in $ALL_TRIO; do
    name="${f#"$SCRIPT_DIR"/}"

    grep -q 'define-spec-pr:begin' "$f" \
        || fail "$name: missing define-spec-pr:begin sentinel (§D4)"
    grep -q 'define-spec-pr:end' "$f" \
        || fail "$name: missing define-spec-pr:end sentinel (§D4)"

    grep -q 'worktree-guard.sh' "$f" \
        || fail "$name: missing worktree-guard.sh reference (§D4)"

    grep -q '/tmp/wt-spec-' "$f" \
        || fail "$name: missing /tmp/wt-spec-<slug> temp worktree path (§D4)"

    grep -q 'worktree-guard\.sh assert\|worktree-guard\.sh.*\n.*assert\|worktree-guard\.sh.*create.*assert\|MUST exit 0 before any edit' "$f" \
        || fail "$name: missing worktree-guard.sh assert call or 'MUST exit 0' directive (§D4)"

    grep -q 'resolve-branch' "$f" \
        || fail "$name: missing resolve-branch call for PR-aware ladder (§D4)"

    grep -q 'git worktree remove' "$f" \
        || fail "$name: missing 'git worktree remove' cleanup (§D4)"

    grep -q 'git worktree prune' "$f" \
        || fail "$name: missing 'git worktree prune' cleanup (§D4)"
done

info "define-trio spec-PR worktree routing present in all 3 files"

# ── 6. AGENTS.md: ## Git hygiene (agents) section ────────────────────────────

[ -f "$AGENTS" ] || fail "AGENTS.md missing at repo root (§D5)"

grep -q '^## Git hygiene (agents)$' "$AGENTS" \
    || fail "AGENTS.md missing '## Git hygiene (agents)' section (§D5)"

grep -q 'Fetch-before-branch\|fetch-before-branch' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing fetch-before-branch rule (§D5)"

grep -q 'primary checkout' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing primary-checkout read-only rule (§D5)"

grep -q 'fresh-or-verified-clean\|Fresh-or-verified-clean' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing fresh-or-verified-clean worktrees rule (§D5)"

grep -q 'git worktree remove' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing 'git worktree remove' cleanup rule (§D5)"

grep -q 'git worktree prune' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing 'git worktree prune' cleanup rule (§D5)"

grep -q 'open-pr\|branch-only' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing PR-aware ladder states (§D5)"

grep -q 'worktree-guard\.sh' "$AGENTS" \
    || fail "AGENTS.md git hygiene section missing pointer to worktree-guard.sh (§D5)"

info "AGENTS.md ## Git hygiene (agents) section present with all 5 sub-rules"

# ── 7. define-spec-pr bash block is well-formed ───────────────────────────────

block="$(awk '
    /<!-- define-spec-pr:begin -->/ { cap=1; next }
    /<!-- define-spec-pr:end -->/   { cap=0 }
    cap && /^```bash/ { code=1; next }
    cap && code && /^```/ { code=0 }
    cap && code { print }
' "$SKILL")"

[ -n "$block" ] \
    || fail "SKILL.md: no bash block inside define-spec-pr sentinel (§D4)"

# Replace placeholder {repo} with a literal string so bash -n doesn't balk.
printf '%s\n' "$block" | sed 's/{repo}/example\/repo/g' | bash -n - \
    || fail "SKILL.md: define-spec-pr bash block fails bash -n (§D4)"

info "define-spec-pr bash block is well-formed"

# ── 8. Lock-step: SKILL.md body == codex/prompt.md == opencode body ──────────

strip_body() { awk '/^---$/{c++; next} c>=2' "$1"; }

diff <(strip_body "$SKILL") <(cat "$CODEX") >/dev/null \
    || fail "autospec-define: SKILL.md body diverges from codex/prompt.md (lock-step)"

diff <(strip_body "$SKILL") <(strip_body "$OPENCODE") >/dev/null \
    || fail "autospec-define: SKILL.md body diverges from opencode/agent.md body (lock-step)"

info "autospec-define trio lock-step byte-identical"

echo "ok: all define spec-PR worktree + AGENTS.md git hygiene checks passed"
