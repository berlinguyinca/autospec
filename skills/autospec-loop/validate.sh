#!/usr/bin/env bash
# skills/autospec-loop/validate.sh — per-skill structural lint for autospec-loop.
#
# Checks:
#   1. Required files exist (trio + install/uninstall/README + scripts + validate.sh).
#   2. Lock-step trio diff: SKILL.md / codex/prompt.md / opencode/agent.md bodies
#      are byte-identical after stripping frontmatter.  Also guards the
#      SKILL.md + codex duo when opencode/agent.md is absent.
#   3. Frontmatter parse: SKILL.md has name + description; opencode/agent.md has
#      description + mode: primary; codex/prompt.md has a leading blank line and
#      no name field.
#   4. bash -n on install.sh, uninstall.sh, and scripts/*.sh.
#   5. Named-content checks: all 7 structural section headers present in each body.
#   6. Loop-logic guards: stop.flag honored, max_iters cap, and separate-verifier
#      no-self-approval clause present in SKILL.md.
#
# Usage:
#   bash skills/autospec-loop/validate.sh [--skill-dir <path>]
#
# Exit 0 = all checks pass. Exit 1 = at least one check failed.

set -eu

fail() { printf 'autospec-loop/validate: FAIL — %s\n' "$*" >&2; exit 1; }
info() { printf 'autospec-loop/validate: %s\n' "$*"; }

SKILL_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --skill-dir) SKILL_DIR="$2"; shift 2 ;;
        *) shift ;;
    esac
done
[ -n "$SKILL_DIR" ] || SKILL_DIR="$(cd "$(dirname "$0")" && pwd)"

SKILL_MD="$SKILL_DIR/SKILL.md"
CODEX_PROMPT="$SKILL_DIR/codex/prompt.md"
OPENCODE_AGENT="$SKILL_DIR/opencode/agent.md"

# ── 1. Required files exist ───────────────────────────────────────────────────
info "checking required files exist"
for f in SKILL.md README.md install.sh uninstall.sh validate.sh \
         opencode/agent.md codex/prompt.md \
         scripts/loop-state.sh scripts/loop-check.sh; do
    [ -f "$SKILL_DIR/$f" ] || fail "missing required file: $f"
done

# ── 2. Lock-step trio diff ─────────────────────────────────────────────────────
# Strip frontmatter (everything between the first two '---' lines) before
# comparing, so the byte-identical body check is not tripped by the per-harness
# frontmatter differences.
strip_frontmatter() {
    # Strip YAML frontmatter (between first two '---' lines) if present, then
    # skip any leading blank lines so bodies are comparable regardless of whether
    # the frontmatter was terminated by a blank line or not.
    # codex/prompt.md has no frontmatter at all — it starts with a leading blank line.
    # Result is the pure body used for lock-step comparison.
    local first_line
    first_line="$(head -1 "$1")"
    if [ "$first_line" = "---" ]; then
        awk 'BEGIN{in_fm=0; done=0; skip_blank=1}
             /^---$/ && !done {
                 in_fm++
                 if (in_fm == 2) { done=1 }
                 next
             }
             done {
                 if (skip_blank && /^[[:space:]]*$/) { next }
                 skip_blank=0
                 print
             }' "$1"
    else
        # No frontmatter — skip the single leading blank line (0x0a)
        awk 'NR==1 && /^[[:space:]]*$/ { next } { print }' "$1"
    fi
}

info "checking lock-step trio diff (SKILL.md / codex/prompt.md / opencode/agent.md)"
TMP_SKILL="$(mktemp)"
TMP_CODEX="$(mktemp)"
TMP_OPENCODE="$(mktemp)"
trap 'rm -f "$TMP_SKILL" "$TMP_CODEX" "$TMP_OPENCODE"' EXIT

strip_frontmatter "$SKILL_MD"    > "$TMP_SKILL"
strip_frontmatter "$CODEX_PROMPT" > "$TMP_CODEX"
strip_frontmatter "$OPENCODE_AGENT" > "$TMP_OPENCODE"

if ! diff -q "$TMP_SKILL" "$TMP_CODEX" >/dev/null 2>&1; then
    fail "lock-step diff: SKILL.md and codex/prompt.md bodies differ"
fi
if ! diff -q "$TMP_SKILL" "$TMP_OPENCODE" >/dev/null 2>&1; then
    fail "lock-step diff: SKILL.md and opencode/agent.md bodies differ"
fi

# ── 3. Frontmatter parse ──────────────────────────────────────────────────────
info "checking SKILL.md frontmatter"
skill_fm="$(awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$SKILL_MD")"
[ -n "$skill_fm" ] || fail "SKILL.md missing or empty YAML frontmatter"
printf '%s\n' "$skill_fm" | grep -q '^name: autospec-loop' \
    || fail "SKILL.md frontmatter missing 'name: autospec-loop'"
printf '%s\n' "$skill_fm" | grep -q '^description:' \
    || fail "SKILL.md frontmatter missing 'description' field"
if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    printf '%s\n' "$skill_fm" | python3 -c "import sys, yaml; yaml.safe_load(sys.stdin.read())" \
        || fail "SKILL.md frontmatter is not valid YAML"
fi

info "checking opencode/agent.md frontmatter"
oc_fm="$(awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$OPENCODE_AGENT")"
[ -n "$oc_fm" ] || fail "opencode/agent.md missing or empty YAML frontmatter"
printf '%s\n' "$oc_fm" | grep -q '^description:' \
    || fail "opencode/agent.md frontmatter missing 'description' field"
printf '%s\n' "$oc_fm" | grep -q '^mode: primary' \
    || fail "opencode/agent.md frontmatter missing 'mode: primary'"
printf '%s\n' "$oc_fm" | grep -q '^name:' \
    && fail "opencode/agent.md frontmatter must NOT have a 'name' field" || true

info "checking codex/prompt.md frontmatter (leading blank line, no name)"
first_char="$(od -An -tx1 -N1 "$CODEX_PROMPT" | tr -d ' \n')"
[ "$first_char" = "0a" ] \
    || fail "codex/prompt.md does not start with a leading blank line (first byte: $first_char, expected 0a)"
codex_fm="$(awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$CODEX_PROMPT")"
if [ -n "$codex_fm" ]; then
    printf '%s\n' "$codex_fm" | grep -q '^name:' \
        && fail "codex/prompt.md frontmatter must NOT have a 'name' field" || true
fi

# ── 4. bash -n on install, uninstall, and scripts ─────────────────────────────
info "checking bash -n on install.sh, uninstall.sh, and scripts/*.sh"
for s in "$SKILL_DIR/install.sh" "$SKILL_DIR/uninstall.sh"; do
    bash -n "$s" || fail "$s: bash syntax error"
done
for s in "$SKILL_DIR/scripts/"*.sh; do
    [ -f "$s" ] || continue
    bash -n "$s" || fail "$s: bash syntax error"
done

# ── 5. Named-content checks: 7 structural section headers ─────────────────────
# Per the shared contracts, these 7 sections must appear in each trio body
# (checked against SKILL.md; lock-step diff above guarantees the others match).
info "checking 7 required structural section headers in each trio body"

SECTIONS=(
    "## Startup self-update"
    "## Self-update mode"
    "## Model tier"
    "## Trigger disambiguation"
    "## Phase 1"
    "## Phase 2"
    "## Guardrails"
)

for section in "${SECTIONS[@]}"; do
    grep -q "^${section}" "$SKILL_MD" \
        || fail "SKILL.md missing required section: '${section}'"
    grep -q "^${section}" "$CODEX_PROMPT" \
        || fail "codex/prompt.md missing required section: '${section}'"
    grep -q "^${section}" "$OPENCODE_AGENT" \
        || fail "opencode/agent.md missing required section: '${section}'"
done

# SKILL_NAME must appear in the Startup self-update block
grep -q 'SKILL_NAME=autospec-loop' "$SKILL_MD" \
    || fail "SKILL.md Startup self-update missing 'SKILL_NAME=autospec-loop'"

# Model tier must reference both tier labels
grep -q 'TIER_A' "$SKILL_MD" \
    || fail "SKILL.md Model tier section missing TIER_A reference"
grep -q 'TIER_B' "$SKILL_MD" \
    || fail "SKILL.md Model tier section missing TIER_B reference"

# Harness adapter table must have the Subagent model tier row
grep -q 'Subagent model tier' "$SKILL_MD" \
    || fail "SKILL.md adapter table missing 'Subagent model tier' row"

# ── 6. Loop-logic guards ──────────────────────────────────────────────────────
info "checking loop-logic guards in SKILL.md"

# stop.flag honored
grep -q 'stop\.flag' "$SKILL_MD" \
    || fail "SKILL.md does not reference 'stop.flag' (guardrail required)"

# max_iters cap
grep -q 'max_iters\|max-iters' "$SKILL_MD" \
    || fail "SKILL.md does not reference max_iters cap (guardrail required)"

# separate-verifier / no-self-approval clause
grep -q 'no.self.approval\|never the worker\|separate.*verifier\|verifier.*separate' "$SKILL_MD" \
    || fail "SKILL.md missing separate-verifier no-self-approval clause"

info "OK — all autospec-loop structural checks passed."
