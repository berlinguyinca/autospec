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

# Stop mode invariants (introduced by the autospec-stop mechanism): every
# multi-harness skill trio must carry a `## Stop mode` heading in all three
# trio files (SKILL.md, opencode/agent.md, codex/prompt.md). Parallels
# check_self_update. Applies to: autospec autospec-run autospec-stop (and
# any future skill that dispatches stop sub-modes).
check_stop_mode_section() {
    for s in autospec autospec-run autospec-stop; do
        skill_dir="skills/$s"
        [ -d "$skill_dir" ] || continue
        info "stop-mode: $s"
        for trio in SKILL.md opencode/agent.md codex/prompt.md; do
            grep -q '^## Stop mode' "$skill_dir/$trio" \
                || fail "$s: $trio missing '## Stop mode' section"
        done
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

# Startup preflight invariants (introduced by issues #115–#119): every
# multi-harness skill must carry a ## Startup self-update section whose bash
# body is byte-identical to the canonical block in skills/autospec/SKILL.md
# (modulo the SKILL_NAME= first line).
check_startup_preflight() {
    extract_block() {
        awk '/^## Startup self-update/{f=1} f && /^```bash/{g=1; next} g && /^```/{g=0; f=0; next} g{print}' "$1" \
            | grep -v '^SKILL_NAME='
    }
    canonical=$(extract_block skills/autospec/SKILL.md)
    [ -n "$canonical" ] || fail "autospec SKILL.md missing ## Startup self-update section"
    for s in autospec autospec-define autospec-run autospec-listen autospec-classify; do
        for f in "skills/$s/SKILL.md" "skills/$s/opencode/agent.md" "skills/$s/codex/prompt.md"; do
            body=$(extract_block "$f")
            [ -n "$body" ] || fail "$f missing ## Startup self-update section"
            if ! diff <(printf '%s\n' "$canonical") <(printf '%s\n' "$body") >/dev/null 2>&1; then
                diff <(printf '%s\n' "$canonical") <(printf '%s\n' "$body") | head -20 >&2
                fail "$f preflight body diverges from canonical"
            fi
        done
    done
}

# Codex skills-dir install invariants (introduced by issues #129–#130): every
# multi-harness skill's install.sh must write SKILL.md to both the legacy
# prompts/ path AND the new skills/ slash-command registry path.
check_codex_skills_install() {
    info "codex skills-dir install: all skills"
    for s in autospec autospec-define autospec-run autospec-listen autospec-classify; do
        f="skills/$s/install.sh"
        grep -q 'skills/\$SKILL_NAME/SKILL\.md' "$f" \
            || fail "$f missing Codex skills-dir install (skills/\$SKILL_NAME/SKILL.md)"
        grep -q 'prompts/\$SKILL_NAME\.md\|CODEX_DEST' "$f" \
            || fail "$f missing legacy Codex prompts-file install"
    done
}

# Two-tier subagent model selection invariants: every dispatching SKILL.md
# must include the literal "**Model tier:**" directive at least once, every
# such directive must specify either "Tier A (spec work)" or
# "Tier B (implementation work)", and the SKILL.md harness-adapter table must
# carry a "Subagent model tier" row. Per-skill Tier-A and Tier-B counts must
# match the expected per-skill values to catch future drift.
check_subagent_model_tier() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "subagent model tier: $name"
    grep -q -F '**Model tier:**' "$skill_dir/SKILL.md" \
        || fail "$name: SKILL.md missing '**Model tier:**' directive on at least one subagent dispatch"
    grep -q -E '^\| Subagent model tier ' "$skill_dir/SKILL.md" \
        || fail "$name: SKILL.md harness-adapter table missing 'Subagent model tier' row"

    # Every **Model tier:** line must label itself Tier A or Tier B.
    bare="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -v -F 'Tier A (spec work)' \
        | grep -v -F 'Tier B (implementation work)' || true)"
    if [ -n "$bare" ]; then
        printf '%s\n' "$bare" >&2
        fail "$name: SKILL.md has '**Model tier:**' directive(s) without 'Tier A (spec work)' or 'Tier B (implementation work)' label"
    fi

    # Per-skill Tier-A / Tier-B count assertions.
    a_count="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -c -F 'Tier A (spec work)' || true)"
    b_count="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -c -F 'Tier B (implementation work)' || true)"
    case "$name" in
        autospec)
            expected_a=4; expected_b=2 ;;
        autospec-define)
            expected_a=3; expected_b=0 ;;
        autospec-run)
            expected_a=1; expected_b=2 ;;
        autospec-classify)
            expected_a=1; expected_b=0 ;;
        *)
            expected_a=""; expected_b="" ;;
    esac
    if [ -n "$expected_a" ]; then
        [ "$a_count" = "$expected_a" ] \
            || fail "$name: expected $expected_a Tier-A dispatches, found $a_count"
        [ "$b_count" = "$expected_b" ] \
            || fail "$name: expected $expected_b Tier-B dispatches, found $b_count"
    fi
}

# AGENTS.md must document the two-tier subagent model selection policy with
# both Tier A and Tier B subsection headings.
check_agents_md_subagent_section() {
    info "AGENTS.md: two-tier subagent model selection section"
    [ -f AGENTS.md ] || fail "AGENTS.md missing at repo root"
    grep -q '^## Subagent model selection (two-tier, cost-aware)$' AGENTS.md \
        || fail "AGENTS.md missing '## Subagent model selection (two-tier, cost-aware)' section"
    grep -q '^### Tier A — Specification work' AGENTS.md \
        || fail "AGENTS.md missing '### Tier A — Specification work' subheading"
    grep -q '^### Tier B — Implementation work' AGENTS.md \
        || fail "AGENTS.md missing '### Tier B — Implementation work' subheading"
}

# Listener skill must ship a complete trio plus references/trigger-keywords.md
# and a README. Per spec §6.1, validate enforces presence of every required
# file so the skill is never partially shipped.
check_autospec_listen_files() {
    info "presence: skills/autospec-listen/"
    listen_dir="skills/autospec-listen"
    [ -d "$listen_dir" ] || fail "$listen_dir: directory missing"
    for f in \
        SKILL.md \
        install.sh \
        uninstall.sh \
        opencode/agent.md \
        codex/prompt.md \
        references/trigger-keywords.md \
        README.md
    do
        [ -f "$listen_dir/$f" ] || fail "$listen_dir/$f: required file missing"
    done
}

# Examples must ship the canonical model-profiles.yml + project-map.yml so
# users can copy them into ~/.autospec/ on day one (spec §6.1, §7.1).
check_examples_dir() {
    info "presence: examples/"
    [ -d examples ] || fail "examples/: directory missing"
    for f in model-profiles.yml project-map.yml README.md; do
        [ -f "examples/$f" ] || fail "examples/$f: required file missing"
    done
}

# Lint-issue helpers invariants (introduced by issue #152): scripts/lint-issue.sh
# must pass bash syntax, --help must print a Usage: line, and the
# tests/fixtures/issue-quality/ directory must contain good.md plus ≥4 bad-*.md
# files. Mirrors the check_self_update pattern.
check_lint_issue_helpers() {
    info "lint-issue helpers: scripts/lint-issue.sh + fixtures"
    [ -f scripts/lint-issue.sh ] \
        || fail "scripts/lint-issue.sh: file missing"
    bash -n scripts/lint-issue.sh \
        || fail "scripts/lint-issue.sh: bash syntax error"
    bash scripts/lint-issue.sh --help 2>/dev/null | grep -q '^Usage:' \
        || fail "scripts/lint-issue.sh --help did not print a 'Usage:' line"
    [ -d tests/fixtures/issue-quality ] \
        || fail "tests/fixtures/issue-quality/: directory missing"
    [ -f tests/fixtures/issue-quality/good.md ] \
        || fail "tests/fixtures/issue-quality/good.md: fixture missing"
    bad_count="$(ls tests/fixtures/issue-quality/bad-*.md 2>/dev/null | wc -l | tr -d ' ')"
    [ "$bad_count" -ge 4 ] \
        || fail "tests/fixtures/issue-quality/: need >=4 bad-*.md fixtures, found ${bad_count}"
}

# Governance copy: the listener lifecycle and anti-loop guardrails must be
# documented in AGENTS.md, and the listener skill must appear in both the
# top-level README.md and SKILLS.md skill index. Spec §6.1, §7.1.
check_governance_headings() {
    info "governance headings: AGENTS.md / README.md / SKILLS.md"
    [ -f AGENTS.md ] || fail "AGENTS.md: file missing at repo root"
    grep -q '^## Anti-loop guardrails' AGENTS.md \
        || fail "AGENTS.md: missing '## Anti-loop guardrails' heading"
    grep -q '^## Listener-filed issues lifecycle' AGENTS.md \
        || fail "AGENTS.md: missing '## Listener-filed issues lifecycle' heading"

    [ -f README.md ] || fail "README.md: file missing at repo root"
    grep -q 'autospec-listen' README.md \
        || fail "README.md: missing 'autospec-listen' reference"

    [ -f SKILLS.md ] || fail "SKILLS.md: file missing at repo root"
    grep -q 'autospec-listen' SKILLS.md \
        || fail "SKILLS.md: missing 'autospec-listen' reference"
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

    check_startup_preflight
    check_stop_mode_section
    check_codex_skills_install

    check_agents_md_subagent_section
    check_autospec_listen_files
    check_examples_dir
    check_governance_headings
    check_lint_issue_helpers

    # Top-level installer / uninstaller (introduced in PR #11) — only check syntax
    # if present; absence is OK before that PR lands.
    check_bash_syntax "install.sh"
    check_bash_syntax "uninstall.sh"

    info "OK — all validation checks passed."
}

main "$@"
