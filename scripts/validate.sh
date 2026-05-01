#!/usr/bin/env bash
# scripts/validate.sh — repository-wide validation harness.
#
# This repo has no language-level test runner. Validation checks:
#   1. Lock-step rule: SKILL.md body == opencode/agent.md body == codex/prompt.md
#      body for every multi-harness skill (after frontmatter is stripped per the
#      project convention: awk '/^---$/{c++; next} c>=2').
#   2. install.sh / uninstall.sh in every skill pass `bash -n` syntax check.
#   3. YAML frontmatter in SKILL.md and opencode/agent.md parses.
#   4. Required files exist for every multi-harness skill.
#
# Exit 0 on success; non-zero on first failure (with a clear diagnostic).

set -eu

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

fail() {
    printf 'validate: FAIL — %s\n' "$*" >&2
    exit 1
}

info() {
    printf 'validate: %s\n' "$*"
}

# Discover multi-harness skills: directories under skills/ that contain SKILL.md
# *and* opencode/agent.md *and* codex/prompt.md.
discover_skills() {
    for d in skills/*/; do
        [ -d "$d" ] || continue
        if [ -f "$d/SKILL.md" ] && [ -f "$d/opencode/agent.md" ] && [ -f "$d/codex/prompt.md" ]; then
            printf '%s\n' "${d%/}"
        fi
    done
}

# Strip frontmatter and any HR --- separator lines per repo convention.
strip_body() {
    awk '/^---$/{c++; next} c>=2' "$1"
}

check_lockstep() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "lock-step: $name"
    if ! diff <(strip_body "$skill_dir/SKILL.md") <(cat "$skill_dir/codex/prompt.md") >/dev/null; then
        diff <(strip_body "$skill_dir/SKILL.md") <(cat "$skill_dir/codex/prompt.md") | head -40 >&2
        fail "$name: SKILL.md body diverges from codex/prompt.md"
    fi
    if ! diff <(strip_body "$skill_dir/SKILL.md") <(strip_body "$skill_dir/opencode/agent.md") >/dev/null; then
        diff <(strip_body "$skill_dir/SKILL.md") <(strip_body "$skill_dir/opencode/agent.md") | head -40 >&2
        fail "$name: SKILL.md body diverges from opencode/agent.md"
    fi
}

check_bash_syntax() {
    target="$1"
    if [ -f "$target" ]; then
        info "bash -n: $target"
        bash -n "$target" || fail "$target: bash syntax error"
    fi
}

extract_frontmatter() {
    awk 'BEGIN{in_fm=0} /^---$/{in_fm++; if(in_fm==1) next; if(in_fm==2) exit} in_fm==1{print}' "$1"
}

check_frontmatter() {
    target="$1"
    info "frontmatter parse: $target"
    fm="$(extract_frontmatter "$target")"
    if [ -z "$fm" ]; then
        fail "$target: missing or empty YAML frontmatter"
    fi
    # Validate via python yaml if available; otherwise minimal sanity (key: value).
    if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
        printf '%s\n' "$fm" | python3 -c "import sys, yaml; yaml.safe_load(sys.stdin.read())" \
            || fail "$target: frontmatter is not valid YAML"
    else
        printf '%s\n' "$fm" | grep -E '^[A-Za-z_][A-Za-z0-9_-]*:' >/dev/null \
            || fail "$target: frontmatter does not look like key: value pairs"
    fi
}

check_required_files() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    for f in SKILL.md README.md install.sh uninstall.sh opencode/agent.md codex/prompt.md; do
        [ -f "$skill_dir/$f" ] || fail "$name: missing required file $f"
    done
}

# Self-update invariants (introduced by issue #10): every multi-harness skill
# must document `## Self-update mode` in all three trio files, and its
# install.sh must accept the `--update` flag.
check_self_update() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "self-update: $name"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Self-update mode' "$skill_dir/$trio" \
            || fail "$name: $trio missing '## Self-update mode' section"
    done
    grep -q -- '--update' "$skill_dir/install.sh" \
        || fail "$name: install.sh missing --update flag handling"
}

# Cost-aware subagent model selection invariants: every dispatching SKILL.md
# must include the literal "**Model tier:**" directive at least once and have
# a "Subagent model tier" row in its harness-adapter table.
check_subagent_model_tier() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "subagent model tier: $name"
    grep -q -F '**Model tier:**' "$skill_dir/SKILL.md" \
        || fail "$name: SKILL.md missing '**Model tier:**' directive on at least one subagent dispatch"
    grep -q -E '^\| Subagent model tier ' "$skill_dir/SKILL.md" \
        || fail "$name: SKILL.md harness-adapter table missing 'Subagent model tier' row"
}

# AGENTS.md must document the cost-aware subagent model selection policy.
check_agents_md_subagent_section() {
    info "AGENTS.md: subagent model selection section"
    [ -f AGENTS.md ] || fail "AGENTS.md missing at repo root"
    grep -q '^## Subagent model selection (cost-aware)$' AGENTS.md \
        || fail "AGENTS.md missing '## Subagent model selection (cost-aware)' section"
}

main() {
    info "scanning multi-harness skills under skills/ ..."
    skills="$(discover_skills)"
    if [ -z "$skills" ]; then
        fail "no multi-harness skills found under skills/"
    fi

    for skill_dir in $skills; do
        check_required_files "$skill_dir"
        check_lockstep "$skill_dir"
        check_frontmatter "$skill_dir/SKILL.md"
        check_frontmatter "$skill_dir/opencode/agent.md"
        check_bash_syntax "$skill_dir/install.sh"
        check_bash_syntax "$skill_dir/uninstall.sh"
        check_self_update "$skill_dir"
        check_subagent_model_tier "$skill_dir"
    done

    check_agents_md_subagent_section

    # Top-level installer / uninstaller (introduced in PR #11) — only check syntax
    # if present; absence is OK before that PR lands.
    check_bash_syntax "install.sh"
    check_bash_syntax "uninstall.sh"

    info "OK — all validation checks passed."
}

main "$@"
