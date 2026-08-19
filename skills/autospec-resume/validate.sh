#!/usr/bin/env bash
# skills/autospec-resume/validate.sh — per-skill structural lint for autospec-resume.
#
# Checks:
#   1. Required files exist (trio + install/uninstall/README + scripts).
#   2. SKILL.md carries the standard new-skill structural sections:
#      ## Startup self-update (with SKILL_NAME=autospec-resume), the
#      harness-detection / Subagent-model-tier section, and the
#      ## Required capabilities & harness adapter row.
#   3. SKILL.md frontmatter is valid YAML with a `name: autospec-resume` field.
#   4. codex/prompt.md starts with a leading blank line (byte-precise).
#   5. resume-scan.sh writes NO run-state comment and adds NO new lock.
#   6. All scripts pass `bash -n`.
#
# Usage:
#   bash skills/autospec-resume/validate.sh [--skill-dir <path>]
#
# Exit 0 = all checks pass. Exit 1 = at least one check failed.

set -eu

fail() { printf 'autospec-resume/validate: FAIL — %s\n' "$*" >&2; exit 1; }
info() { printf 'autospec-resume/validate: %s\n' "$*"; }

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
SCAN="$SKILL_DIR/scripts/resume-scan.sh"
REPO_ROOT="$(cd "$SKILL_DIR/../.." && pwd)"

# ── 1. Required files exist ───────────────────────────────────────────────────
info "checking required files exist"
for f in SKILL.md README.md install.sh uninstall.sh validate.sh \
         opencode/agent.md codex/prompt.md \
         scripts/resume-scan.sh scripts/resume-attempts.sh; do
    [ -f "$SKILL_DIR/$f" ] || fail "missing required file: $f"
done
[ -f "$REPO_ROOT/scripts/autospec-run-registry.sh" ] \
    || fail "missing required file: scripts/autospec-run-registry.sh"

# ── 2. Standard new-skill structural sections ─────────────────────────────────
info "checking standard structural sections in SKILL.md"
# Transition-safe (D3a sweep, #1032/#1035): accept EITHER the canonical
# '## Startup self-update' section OR its autospec-block marker form (which
# carries SKILL_NAME=autospec-resume inline). Files with NEITHER fail closed.
if grep -q '^## Startup self-update' "$SKILL_MD"; then
    grep -q 'SKILL_NAME=autospec-resume' "$SKILL_MD" \
        || fail "SKILL.md Startup self-update missing 'SKILL_NAME=autospec-resume'"
elif grep -q 'autospec-block:startup-self-update' "$SKILL_MD"; then
    grep -q 'autospec-block:startup-self-update SKILL_NAME=autospec-resume' "$SKILL_MD" \
        || fail "SKILL.md startup-self-update marker missing 'SKILL_NAME=autospec-resume'"
else
    fail "SKILL.md missing '## Startup self-update' section or autospec-block:startup-self-update marker"
fi
grep -q '^## Harness detection' "$SKILL_MD" \
    || fail "SKILL.md missing harness-detection section"
grep -q 'TIER_A' "$SKILL_MD" \
    || fail "SKILL.md harness-detection missing TIER_A reference"
grep -q 'TIER_B' "$SKILL_MD" \
    || fail "SKILL.md harness-detection missing TIER_B reference"
grep -q '^## Required capabilities & harness adapter' "$SKILL_MD" \
    || fail "SKILL.md missing '## Required capabilities & harness adapter' section"
grep -q 'Subagent model tier' "$SKILL_MD" \
    || fail "SKILL.md adapter table missing 'Subagent model tier' row"

# ── 3. Frontmatter ────────────────────────────────────────────────────────────
info "checking SKILL.md frontmatter"
fm="$(awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$SKILL_MD")"
[ -n "$fm" ] || fail "SKILL.md missing or empty YAML frontmatter"
printf '%s\n' "$fm" | grep -q '^name: autospec-resume' \
    || fail "SKILL.md frontmatter missing 'name: autospec-resume'"
if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    printf '%s\n' "$fm" | python3 -c "import sys, yaml; yaml.safe_load(sys.stdin.read())" \
        || fail "SKILL.md frontmatter is not valid YAML"
fi

# ── 4. codex/prompt.md leading blank line ─────────────────────────────────────
info "checking codex/prompt.md leading blank line"
first_char="$(od -An -tx1 -N1 "$CODEX_PROMPT" | tr -d ' \n')"
[ "$first_char" = "0a" ] \
    || fail "codex/prompt.md does not start with a leading blank line (first byte: $first_char, expected 0a)"

# ── 5. resume-scan.sh writes no run-state comment / adds no new lock ──────────
info "checking resume-scan.sh adds no second lock"
if grep -E 'autospec[[:space:]]+claim[[:space:]]+state[[:space:]]+(upsert|clear)|gh issue comment' \
        "$SCAN"; then
    fail "resume-scan.sh must NOT write a run-state comment or add a lock (found a mutation call)"
fi
grep -q 'claim state read' "$SCAN" \
    || fail "resume-scan.sh must READ run-state through autospec claim, never re-implement it"
grep -q 'claim state recover-stale-startup' "$SCAN" \
    || fail "resume-scan.sh must delegate stale-startup recovery to autospec claim state recover-stale-startup"
while IFS= read -r claim_state_command; do
    case "$claim_state_command" in
        read|recover-stale-startup) ;;
        *) fail "resume-scan.sh uses forbidden claim state command: $claim_state_command" ;;
    esac
done < <(sed -nE 's/.*claim[[:space:]]+state[[:space:]]+([[:alnum:]-]+).*/\1/p' "$SCAN")

dry_line="$(grep -n 'if \[ "$DRY_RUN" -eq 1 \]' "$SCAN" | cut -d: -f1)"
recover_line="$(grep -n 'claim state recover-stale-startup' "$SCAN" | cut -d: -f1)"
[ -n "$dry_line" ] || fail "resume-scan.sh missing dry-run boundary"
[ "$recover_line" -gt "$dry_line" ] \
    || fail "stale-startup recovery must follow the dry-run boundary"

# ── 6. bash -n on all scripts ─────────────────────────────────────────────────
info "checking bash -n on all scripts"
for s in "$SKILL_DIR/scripts/resume-scan.sh" \
         "$SKILL_DIR/scripts/resume-attempts.sh" \
         "$REPO_ROOT/scripts/autospec-run-registry.sh"; do
    bash -n "$s" || fail "$s: bash syntax error"
    [ -x "$s" ] || fail "$s: not executable"
done

info "OK — all autospec-resume structural checks passed."
