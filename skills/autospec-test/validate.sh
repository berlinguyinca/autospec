#!/usr/bin/env bash
# skills/autospec-test/validate.sh — per-skill structural lint for autospec-test.
#
# Checks:
#   1. SKILL.md contains every required structural section in correct order.
#   2. SKILL.md frontmatter is valid YAML with a `name:` field.
#   3. SKILL.md has no {FEATURE_DESCRIPTION} shell-out placeholders (pure-prose rule).
#   4. codex/prompt.md starts with a leading blank line (byte-precise).
#   5. SKILL.md contains a forbidden_url_patterns example block (adapter row).
#   6. SKILL.md Model tier declares reasoning:standard and ctx:120k.
#
# Usage:
#   bash skills/autospec-test/validate.sh [--skill-dir <path>]
#
# Exit 0 = all checks pass.
# Exit 1 = at least one check failed (diagnostic printed to stderr).

set -eu

fail() {
    printf 'autospec-test/validate: FAIL — %s\n' "$*" >&2
    exit 1
}

info() {
    printf 'autospec-test/validate: %s\n' "$*"
}

# Resolve skill dir: --skill-dir <path> overrides the default (this script's parent).
SKILL_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --skill-dir)
            SKILL_DIR="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done
if [ -z "$SKILL_DIR" ]; then
    SKILL_DIR="$(cd "$(dirname "$0")" && pwd)"
fi

SKILL_MD="$SKILL_DIR/SKILL.md"
CODEX_PROMPT="$SKILL_DIR/codex/prompt.md"

# ── 1. Required files exist ───────────────────────────────────────────────────
info "checking required files exist"
[ -f "$SKILL_MD" ]      || fail "SKILL.md missing at $SKILL_MD"
[ -f "$CODEX_PROMPT" ]  || fail "codex/prompt.md missing at $CODEX_PROMPT"

# ── 2. SKILL.md: required structural sections (in order) ─────────────────────
info "checking required structural sections in SKILL.md"

REQUIRED_SECTIONS=(
    "## Self-update"
    "## Model tier"
    "## When to use"
    "## When not to use"
    "## How it works"
    "## Contract file"
    "## Modes I and II"
    "## Safety rails"
    "## Self-heal loop"
    "## Wizard"
    "## Stop mode"
)

last_line=0
for section in "${REQUIRED_SECTIONS[@]}"; do
    line_num=$(grep -n "^${section}" "$SKILL_MD" | head -1 | cut -d: -f1)
    if [ -z "$line_num" ]; then
        fail "SKILL.md missing required section: '$section'"
    fi
    if [ "$line_num" -le "$last_line" ]; then
        fail "SKILL.md section '$section' appears out of order (line $line_num, expected after $last_line)"
    fi
    last_line="$line_num"
done
info "all required sections present and in order"

# ── 3. SKILL.md: Model tier declares reasoning:standard and ctx:120k ─────────
info "checking Model tier declarations in SKILL.md"
grep -q 'reasoning:standard' "$SKILL_MD" \
    || fail "SKILL.md Model tier missing 'reasoning:standard' declaration"
grep -q 'ctx:120k' "$SKILL_MD" \
    || fail "SKILL.md Model tier missing 'ctx:120k' declaration"

# ── 4. SKILL.md: No {FEATURE_DESCRIPTION} shell-out (pure-prose rule) ────────
info "checking for forbidden FEATURE_DESCRIPTION placeholder in SKILL.md"
if grep -q '{FEATURE_DESCRIPTION}' "$SKILL_MD"; then
    fail "SKILL.md contains forbidden '{FEATURE_DESCRIPTION}' placeholder (Stop/Self-update sections must be pure prose)"
fi

# ── 5. SKILL.md: forbidden_url_patterns example block (adapter row) ──────────
info "checking forbidden_url_patterns example block in SKILL.md"
grep -q 'forbidden_url_patterns' "$SKILL_MD" \
    || fail "SKILL.md missing forbidden_url_patterns example block (required adapter row content)"

# ── 6. SKILL.md: YAML frontmatter with name: field ───────────────────────────
info "checking SKILL.md frontmatter"
fm="$(awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$SKILL_MD")"
if [ -z "$fm" ]; then
    fail "SKILL.md missing or empty YAML frontmatter"
fi
printf '%s\n' "$fm" | grep -q '^name:' \
    || fail "SKILL.md frontmatter missing 'name:' field"
if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    printf '%s\n' "$fm" | python3 -c "import sys, yaml; yaml.safe_load(sys.stdin.read())" \
        || fail "SKILL.md frontmatter is not valid YAML"
fi

# ── 7. codex/prompt.md: leading blank line (byte-precise) ────────────────────
info "checking codex/prompt.md leading blank line"
first_char="$(head -c 1 "$CODEX_PROMPT" | xxd -p 2>/dev/null || od -An -tx1 -N1 "$CODEX_PROMPT" | tr -d ' \n')"
if [ "$first_char" != "0a" ]; then
    fail "codex/prompt.md does not start with a leading blank line (first byte: $first_char, expected 0a)"
fi

# ── 8. SKILL.md: v2 invariants_v2 example block ─────────────────────────────
info "checking invariants_v2 example block in SKILL.md"
grep -q 'invariants_v2:' "$SKILL_MD" \
    || fail "SKILL.md missing 'invariants_v2:' example block (required for v2 lockstep)"

# ── 9. SKILL.md: edge_case_seeds example block ───────────────────────────────
info "checking edge_case_seeds example block in SKILL.md"
grep -q 'edge_case_seeds:' "$SKILL_MD" \
    || fail "SKILL.md missing 'edge_case_seeds:' example block (required for v2 lockstep)"

# ── 10. SKILL.md: v2 adapter row present (Stage 2.5 metrics table) ───────────
info "checking v2 adapter row in SKILL.md"
grep -q 'Stage 2\.5' "$SKILL_MD" \
    || fail "SKILL.md missing Stage 2.5 adapter row (required v2 lockstep check)"

info "OK — all autospec-test structural checks passed."
