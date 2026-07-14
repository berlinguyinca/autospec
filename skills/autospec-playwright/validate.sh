#!/usr/bin/env bash
# validate.sh — per-skill structural lint for autospec-playwright.
# Called by the direct Rust validation check_autospec_playwright_skill_present gate.
# Also runnable standalone from the skill directory or the repo root.
#
# Exits 0 on success, 1 on first failure.

set -eu

SKILL_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_NAME="autospec-playwright"

fail() { printf 'validate(%s): FAIL — %s\n' "$SKILL_NAME" "$*" >&2; exit 1; }
info() { printf 'validate(%s): %s\n' "$SKILL_NAME" "$*"; }

# ── Required files ────────────────────────────────────────────────────────────
for f in SKILL.md codex/prompt.md opencode/agent.md install.sh uninstall.sh README.md; do
    [ -f "$SKILL_DIR/$f" ] || fail "missing required file: $f"
done
info "required files present"

# ── Lockstep: SKILL.md body == codex/prompt.md (leading blank line stripped) ─
strip_body() {
    awk '/^---$/{c++; next} c>=2' "$1"
}

skill_body="$(strip_body "$SKILL_DIR/SKILL.md")"
codex_body="$(cat "$SKILL_DIR/codex/prompt.md")"
opencode_body="$(strip_body "$SKILL_DIR/opencode/agent.md")"

# codex/prompt.md must start with a newline (leading blank line)
first_byte="$(head -c1 "$SKILL_DIR/codex/prompt.md" | od -An -c | tr -d ' \n')"
[ "$first_byte" = "\\n" ] || fail "codex/prompt.md must start with a leading blank line (first byte must be newline)"
info "codex/prompt.md leading blank line: ok"

# codex/prompt.md is compared as-is (the repo-level check_lockstep does cat codex/prompt.md)
if ! diff <(printf '%s\n' "$skill_body") <(printf '%s\n' "$codex_body") >/dev/null 2>&1; then
    diff <(printf '%s\n' "$skill_body") <(printf '%s\n' "$codex_body") | head -20 >&2
    fail "SKILL.md body diverges from codex/prompt.md"
fi
info "lockstep SKILL.md == codex/prompt.md: ok"

if ! diff <(printf '%s\n' "$skill_body") <(printf '%s\n' "$opencode_body") >/dev/null 2>&1; then
    diff <(printf '%s\n' "$skill_body") <(printf '%s\n' "$opencode_body") | head -20 >&2
    fail "SKILL.md body diverges from opencode/agent.md"
fi
info "lockstep SKILL.md == opencode/agent.md: ok"

# ── SKILL.md required sections ───────────────────────────────────────────────
grep -q '^## Self-update mode' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md missing '## Self-update mode' section"
info "Self-update mode section: ok"

grep -qF '**Model tier:**' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md missing '**Model tier:**' directive"
info "Model tier directive: ok"

grep -q '^| Subagent model tier' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md missing 'Subagent model tier' row in harness-adapter table"
info "Subagent model tier row: ok"

grep -q '^## Harness detection' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md missing '## Harness detection' section"
grep -q 'TIER_A' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md Harness detection missing TIER_A"
grep -q 'TIER_B' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md Harness detection missing TIER_B"
grep -qi 'silently' "$SKILL_DIR/SKILL.md" \
    || fail "SKILL.md Harness detection missing 'silently' fallback"
info "Harness detection section: ok"

# ── install.sh has --update ───────────────────────────────────────────────────
grep -q -- '--update' "$SKILL_DIR/install.sh" \
    || fail "install.sh missing --update flag handling"
info "install.sh --update: ok"

# ── bash syntax ───────────────────────────────────────────────────────────────
bash -n "$SKILL_DIR/install.sh"   || fail "install.sh: bash syntax error"
bash -n "$SKILL_DIR/uninstall.sh" || fail "uninstall.sh: bash syntax error"
info "bash syntax: ok"

info "all checks passed"
exit 0
