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

# Discover duo-harness skills: directories under skills/ that contain SKILL.md
# *and* codex/prompt.md but NOT opencode/agent.md.
discover_duo_skills() {
    for d in skills/*/; do
        [ -d "$d" ] || continue
        if [ -f "$d/SKILL.md" ] && [ -f "$d/codex/prompt.md" ] && [ ! -f "$d/opencode/agent.md" ]; then
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

# Duo-mode lockstep: byte-diff SKILL.md body against codex/prompt.md when
# opencode/agent.md is absent. Fails with a message naming both files.
check_lockstep_duo() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "lock-step (duo): $name"
    if ! diff <(strip_body "$skill_dir/SKILL.md") <(cat "$skill_dir/codex/prompt.md") >/dev/null; then
        diff <(strip_body "$skill_dir/SKILL.md") <(cat "$skill_dir/codex/prompt.md") | head -40 >&2
        fail "$name: $skill_dir/SKILL.md body diverges from $skill_dir/codex/prompt.md"
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

# Keyword auto-routing invariants (issue #538): the autospec-listen trio must
# carry a `## Keyword auto-routing` heading in all three trio files
# (SKILL.md, opencode/agent.md, codex/prompt.md) so the broadened verb->skill
# routing behavior stays in lockstep. Parallels check_stop_mode_section.
check_keyword_routing_section() {
    skill_dir="skills/autospec-listen"
    [ -d "$skill_dir" ] || return 0
    info "keyword-routing: autospec-listen"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Keyword auto-routing' "$skill_dir/$trio" \
            || fail "autospec-listen: $trio missing '## Keyword auto-routing' section"
    done
}

# Gap-remediation invariants (issue #535, deps #533/#534): the autospec-run
# trio must carry a `## Phase 5.5 — End-of-run gap remediation` heading in all
# three trio files (SKILL.md, opencode/agent.md, codex/prompt.md) so the
# end-of-run gap-remediation loop stays in lockstep. Parallels
# check_stop_mode_section.
check_gap_remediation_section() {
    for s in autospec-run; do
        skill_dir="skills/$s"
        [ -d "$skill_dir" ] || continue
        info "gap-remediation: $s"
        for trio in SKILL.md opencode/agent.md codex/prompt.md; do
            grep -q '^## Phase 5[.]5 — End-of-run gap remediation' "$skill_dir/$trio" \
                || fail "$s: $trio missing '## Phase 5.5 — End-of-run gap remediation' section"
        done
    done
}

# Remediation-mode invariants (issue #535, dep #533): the autospec-review trio
# must carry a `## Remediation mode` heading in all three trio files
# (SKILL.md, opencode/agent.md, codex/prompt.md) so the broad-review/gap-emit
# remediation mode stays in lockstep. Parallels check_stop_mode_section.
check_review_remediation_section() {
    skill_dir="skills/autospec-review"
    [ -d "$skill_dir" ] || return 0
    info "review-remediation: autospec-review"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Remediation mode' "$skill_dir/$trio" \
            || fail "autospec-review: $trio missing '## Remediation mode' section"
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
    for s in autospec autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop; do
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
    for s in autospec autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop; do
        f="skills/$s/install.sh"
        grep -q 'skills/\$SKILL_NAME/SKILL\.md' "$f" \
            || fail "$f missing Codex skills-dir install (skills/\$SKILL_NAME/SKILL.md)"
        grep -q 'prompts/\$SKILL_NAME\.md\|CODEX_DEST' "$f" \
            || fail "$f missing legacy Codex prompts-file install"
    done
}

# Every per-skill installer must install shared helper scripts into
# ~/.autospec/scripts so runtime commands work in target repositories that do
# not contain this repo's scripts/ directory.
check_shared_script_install() {
    info "shared helper install: all skills"
    helpers="autospec-stop.sh autospec-watchdog.sh autospec-watchdog.ps1 lint-implementation.sh lint-issue.sh listener-match.sh sizing-check.sh"
    for s in autospec autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop; do
        f="skills/$s/install.sh"
        grep -q 'install_shared_scripts' "$f" \
            || fail "$f missing install_shared_scripts function/call"
        grep -q '\.autospec/scripts' "$f" \
            || fail "$f missing ~/.autospec/scripts install target"
        for helper in $helpers; do
            grep -q "$helper" "$f" \
                || fail "$f missing shared helper install entry: $helper"
        done
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
    # Accept EITHER the legacy verbose format ("Tier A (spec work)" / "Tier B (implementation work)")
    # OR the new TIER_A / TIER_B reference format introduced by the harness-aware model-tier design.
    bare="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -v -F 'Tier A (spec work)' \
        | grep -v -F 'Tier B (implementation work)' \
        | grep -v -E 'TIER_[AB]' || true)"
    if [ -n "$bare" ]; then
        printf '%s\n' "$bare" >&2
        fail "$name: SKILL.md has '**Model tier:**' directive(s) without 'Tier A (spec work)', 'Tier B (implementation work)', or TIER_A/TIER_B label"
    fi

    # Per-skill Tier-A / Tier-B count assertions — only enforced for legacy format.
    # Skills using the new TIER_A/TIER_B reference format skip the hard count check.
    if grep -q -E 'TIER_[AB]' "$skill_dir/SKILL.md"; then
        return 0
    fi

    a_count="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -c -F 'Tier A (spec work)' || true)"
    b_count="$(grep -F '**Model tier:**' "$skill_dir/SKILL.md" \
        | grep -c -F 'Tier B (implementation work)' || true)"
    case "$name" in
        autospec)
            expected_a=4; expected_b=2 ;;
        autospec-define|autospec-split)
            expected_a=3; expected_b=0 ;;
        autospec-run)
            expected_a=2; expected_b=3 ;;
        autospec-classify)
            expected_a=1; expected_b=0 ;;
        autospec-story)
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

# Harness detection block validation.
# If a SKILL.md contains a "## Harness detection" heading, verify that the
# section body includes TIER_A, TIER_B, and a "silently" fallback reference.
# If the heading is absent, emit a WARN (not FAIL) to allow incremental migration.
check_harness_detection_block() {
    file="$1"
    name="$(basename "$(dirname "$file")")"
    if ! grep -q '## Harness detection' "$file"; then
        printf 'validate: WARN — %s: missing ## Harness detection section (incremental migration OK)\n' "$name" >&2
        return 0
    fi
    info "harness detection block: $name"
    grep -q 'TIER_A' "$file" \
        || fail "$name: ## Harness detection section present but missing TIER_A reference"
    grep -q 'TIER_B' "$file" \
        || fail "$name: ## Harness detection section present but missing TIER_B reference"
    grep -q -i 'silently' "$file" \
        || fail "$name: ## Harness detection section present but missing 'silently' fallback reference"
}

check_monitor_batch_exit() {
    local file="$1"
    # Only enforce on skills that contain a Phase 4 monitor outer loop.
    # Skills without Phase 4 (e.g. autospec-classify, autospec-review) are silently skipped.
    if ! grep -q "^## Phase 4 — Background autonomous monitor" "$file" 2>/dev/null; then
        return 0
    fi
    local name
    name="$(basename "$(dirname "$file")")"
    local missing=()
    grep -q "batch_issue_count" "$file"   || missing+=("batch_issue_count")
    grep -q "AUTOSPEC_BATCH_SIZE"  "$file" || missing+=("AUTOSPEC_BATCH_SIZE")
    grep -q "batch-done.json"      "$file" || missing+=("batch-done.json")
    grep -q "BATCH_COMPLETE"       "$file" || missing+=("BATCH_COMPLETE")
    grep -q "ALL_DONE"             "$file" || missing+=("ALL_DONE")
    if [ "${#missing[@]}" -gt 0 ]; then
        fail "$name: monitor batch-exit missing: ${missing[*]}"
    fi
    info "monitor-batch-exit: $name"
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

# Lint-implementation helpers invariants (introduced by issue #212):
# scripts/lint-implementation.sh must exist, be executable, pass bash -n, and
# --help must list all 10 RULE_IDs. Mirrors the check_lint_issue_helpers pattern.
check_lint_implementation_helpers() {
    info "lint-implementation helpers: scripts/lint-implementation.sh"
    [ -f scripts/lint-implementation.sh ] \
        || fail "scripts/lint-implementation.sh: file missing"
    bash -n scripts/lint-implementation.sh \
        || fail "scripts/lint-implementation.sh: bash syntax error"
    help_out="$(bash scripts/lint-implementation.sh --help 2>&1)"
    for rule_id in OUT_OF_SCOPE MISSING_TEST COMPLEXITY SECURITY TODO_LEFT MOCK_DB \
                   DOC_OUT_OF_SYNC HALLUCINATED_API DUPLICATE_CODE INVENTED_CONFIG; do
        printf '%s\n' "$help_out" | grep -q "$rule_id" \
            || fail "scripts/lint-implementation.sh --help missing RULE_ID: $rule_id"
    done
}

# Phase 4 guardian block lock-step invariants (introduced by issue #212): the
# outer guardian dispatch block (between <!-- guardian-block:begin --> and
# <!-- guardian-block:end --> markers) must be byte-identical across all 6
# Phase 4 trio files (autospec + autospec-run × SKILL.md/codex/opencode).
# Per spec §5.2 lock-step trio.
check_phase4_guardian_block_lockstep() {
    info "phase4 guardian block lockstep: autospec + autospec-run trio"
    extract_guardian_block() {
        awk '
            /<!-- guardian-block:begin -->/ { depth++; if (depth == 1) { next } }
            /<!-- guardian-block:end -->/   { if (depth == 1) { depth=0; next } depth-- }
            depth >= 1 { print }
        ' "$1"
    }
    # Use autospec SKILL.md as the canonical reference
    canonical_file="skills/autospec/SKILL.md"
    [ -f "$canonical_file" ] || fail "guardian lockstep: $canonical_file missing"
    canonical="$(extract_guardian_block "$canonical_file")"
    [ -n "$canonical" ] \
        || fail "guardian lockstep: no guardian block found in $canonical_file"
    for trio_file in \
        skills/autospec/codex/prompt.md \
        skills/autospec/opencode/agent.md \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
    do
        [ -f "$trio_file" ] || fail "guardian lockstep: $trio_file missing"
        if ! diff -q \
            <(printf '%s\n' "$canonical") \
            <(extract_guardian_block "$trio_file") \
            >/dev/null 2>&1; then
            diff \
                <(printf '%s\n' "$canonical") \
                <(extract_guardian_block "$trio_file") | head -20 >&2
            fail "guardian lockstep: $trio_file guardian block diverges from $canonical_file"
        fi
    done
    info "guardian lockstep: all 6 trio files byte-identical"
}

# Phase 4 issue-start visibility: every monitor surface that can process
# auto-implement issues must print a concise issue summary after claim and
# before `process(ISSUE)` dispatch. This keeps long-running monitors observable.
check_phase4_issue_start_summary() {
    info "phase4 issue-start summary: autospec + autospec-run trio"
    for s in autospec autospec-run; do
        for f in "skills/$s/SKILL.md" "skills/$s/opencode/agent.md" "skills/$s/codex/prompt.md"; do
            grep -q 'Issue start summary' "$f" \
                || fail "$f missing Issue start summary directive"
            grep -q '\[monitor\] starting #\$ISSUE:' "$f" \
                || fail "$f missing [monitor] starting issue summary line"
            grep -q '\[monitor\] goal:' "$f" \
                || fail "$f missing [monitor] goal summary line"
            grep -q '\[monitor\] smoke:' "$f" \
                || fail "$f missing [monitor] smoke summary line"
            grep -q '\[monitor\] scope:' "$f" \
                || fail "$f missing [monitor] scope summary line"
        done
    done
}

# Phase 4 cascade latency: after `process(ISSUE)` returns, the monitor must
# immediately re-enter candidate selection. Sleeping after a completed issue
# delays unblocked follow-up issues and slows recovery from failures.
check_phase4_immediate_next_issue_pickup() {
    info "phase4 immediate next-issue pickup: autospec + autospec-run trio"
    for s in autospec autospec-run; do
        for f in "skills/$s/SKILL.md" "skills/$s/opencode/agent.md" "skills/$s/codex/prompt.md"; do
            grep -q 'Immediate next-issue pickup: NO SLEEP after process(ISSUE)' "$f" \
                || fail "$f missing immediate next-issue pickup directive"
            grep -q 'fresh queue scan can pick any issue unblocked' "$f" \
                || fail "$f missing fresh queue scan/unblocked issue rationale"
        done
    done
}

# Phase 4 adaptive-retry loop: the implementer prompt block must contain the
# MAX_IMPL_RETRIES while-loop with directive_context accumulation and an
# exhaustion branch that posts a manual-intervention comment. All three trio
# files (SKILL.md, codex/prompt.md, opencode/agent.md) must be in sync.
check_phase4_adaptive_retry() {
    info "phase4 adaptive-retry loop: autospec-run trio"
    for f in "skills/autospec-run/SKILL.md" "skills/autospec-run/codex/prompt.md" "skills/autospec-run/opencode/agent.md"; do
        grep -q 'RETRY-LOOP:begin' "$f" \
            || fail "$f missing RETRY-LOOP:begin marker"
        grep -q 'MAX_IMPL_RETRIES' "$f" \
            || fail "$f missing MAX_IMPL_RETRIES variable"
        grep -q 'directive_context' "$f" \
            || fail "$f missing directive_context variable"
        grep -q 'Retry attempt' "$f" \
            || fail "$f missing 'Retry attempt' findings block"
        grep -q 'Implementer hit max retries; manual intervention needed' "$f" \
            || fail "$f missing manual-intervention exhaustion comment"
        grep -q 'auto-implement-active' "$f" \
            || fail "$f missing lock-label release on exhaustion"
    done
}

# Team-personality contract: autospec should infer the right solving team from
# the request, repository evidence, memories, and prior specs. If confidence is
# low, it must ask explicitly with five concrete starter combinations, carry
# the choice into the design spec, include it in child issues, and teach Phase 4
# implementers to honor it as the execution lens.
check_team_personality_contract() {
    info "team personality contract: autospec / autospec-define / autospec-run"
    for f in "skills/autospec/SKILL.md" "skills/autospec-define/SKILL.md"; do
        grep -q '## Team personality selection' "$f" \
            || fail "$f missing Team personality selection section"
        grep -q 'past specs' "$f" \
            || fail "$f missing past-spec inference input"
        grep -q 'confidence is low' "$f" \
            || fail "$f missing low-confidence ask rule"
        grep -q 'five starter combinations' "$f" \
            || fail "$f missing five starter combinations rule"
        grep -q 'Core product engineering' "$f" \
            || fail "$f missing Core product engineering starter team"
        grep -q 'Security-sensitive' "$f" \
            || fail "$f missing Security-sensitive starter team"
        grep -q 'Team personality' "$f" \
            || fail "$f missing Team personality issue/spec section"
        grep -q 'Review counter-team' "$f" \
            || fail "$f missing Review counter-team issue/spec section"
        grep -q 'challenge likely blind spots' "$f" \
            || fail "$f missing counter-team blind-spot directive"
    done
    for f in "skills/autospec/SKILL.md" "skills/autospec-define/SKILL.md" "skills/autospec-split/SKILL.md"; do
        grep -q -- '- \*\*Team personality\*\*' "$f" \
            || fail "$f missing Team personality child issue section"
        grep -q -- '- \*\*Review counter-team\*\*' "$f" \
            || fail "$f missing Review counter-team child issue section"
        grep -q 'body should already contain `## Files to read first`, `## Implementation scope`, `## Team personality`, and `## Review counter-team`' "$f" \
            || fail "$f missing Phase 3.5 team-lens template gate"
    done
    for f in "skills/autospec/SKILL.md" "skills/autospec-run/SKILL.md"; do
        grep -q '## Team personality as execution lens' "$f" \
            || fail "$f missing Phase 4 team personality execution lens"
        grep -q '## Review counter-team as review lens' "$f" \
            || fail "$f missing Phase 4 review counter-team lens"
    done
    grep -q 'team_personality' scripts/gen-issue-skeleton.sh \
        || fail "scripts/gen-issue-skeleton.sh missing team_personality input"
    grep -q 'review_counter_team' scripts/gen-issue-skeleton.sh \
        || fail "scripts/gen-issue-skeleton.sh missing review_counter_team input"
    grep -q -- '--issue-body' scripts/gen-reviewer-prompt.sh \
        || fail "scripts/gen-reviewer-prompt.sh missing --issue-body support"
    grep -q -- '--issue-body "$_body_file"' skills/autospec-run/SKILL.md \
        || fail "skills/autospec-run/SKILL.md reviewer prompt does not pass issue body"
    grep -q 'team_personality' docs/API_REFERENCE.md \
        || fail "docs/API_REFERENCE.md missing gen-issue-skeleton team_personality schema"
    grep -q 'review_counter_team' docs/API_REFERENCE.md \
        || fail "docs/API_REFERENCE.md missing gen-issue-skeleton review_counter_team schema"
    grep -q 'Team personality' skills/autospec/README.md \
        || fail "skills/autospec/README.md missing Team personality docs"
    grep -q 'Review counter-team' skills/autospec-define/README.md \
        || fail "skills/autospec-define/README.md missing Review counter-team docs"
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
    grep -q 'autospec-story' README.md \
        || fail "README.md: missing 'autospec-story' reference"

    [ -f SKILLS.md ] || fail "SKILLS.md: file missing at repo root"
    grep -q 'autospec-listen' SKILLS.md \
        || fail "SKILLS.md: missing 'autospec-listen' reference"
    grep -q 'autospec-story' SKILLS.md \
        || fail "SKILLS.md: missing 'autospec-story' reference"
}

# Existing spec mode invariants: autospec, autospec-split, and autospec-define must expose the
# shortcut that skips Phase 1/2 and reuses Phase 3 + Phase 3.5 for a tracked
# docs/specs/*.md file. Enforce all trio files so harness variants cannot drift.
check_autospec_run_priority_sort_lockstep() {
    info "autospec-run priority sort lockstep"
    diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md | grep -A 8 'Queue priority sort') \
         <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/opencode/agent.md | grep -A 8 'Queue priority sort') \
        || fail "priority sort lockstep (opencode)"
    diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md | grep -A 8 'Queue priority sort') \
         <(sed '1{/^$/d;}' skills/autospec-run/codex/prompt.md | grep -A 8 'Queue priority sort') \
        || fail "priority sort lockstep (codex)"
}

check_autospec_run_regression_review_lockstep() {
    info "autospec-run regression review lockstep"
    for variant in "skills/autospec-run/opencode/agent.md" "skills/autospec-run/codex/prompt.md"; do
        grep -qE "Regression (review escalation|meta-review)" "$variant" \
            || fail "regression review block missing in $variant"
        grep -qE "Tier A \(spec work\)|TIER_A" "$variant" \
            || fail "Tier A annotation missing in $variant"
    done
}

check_autospec_review_skill_present() {
    info "autospec-review skill present + lockstep"
    for f in "skills/autospec-review/SKILL.md" \
             "skills/autospec-review/opencode/agent.md" \
             "skills/autospec-review/codex/prompt.md"; do
        [ -f "$f" ] || fail "$f missing"
    done
    diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
         <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md) \
        || fail "autospec-review opencode lockstep"
    diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
         <(cat skills/autospec-review/codex/prompt.md) \
        || fail "autospec-review codex lockstep"
}

check_autospec_review_tier_a_directives() {
    info "autospec-review Tier A directives"
    grep -q "Tier A (spec work)" skills/autospec-review/SKILL.md \
        || fail "missing 'Tier A (spec work)' directive in autospec-review/SKILL.md"
    count=$(grep -c "Tier A (spec work)" skills/autospec-review/SKILL.md)
    [ "$count" -ge 2 ] \
        || fail "expected ≥2 'Tier A (spec work)' directives in autospec-review/SKILL.md, found $count"
}

check_existing_spec_mode() {
    info "existing-spec mode: autospec + autospec-split + autospec-define"
    for s in autospec autospec-split autospec-define; do
        for f in "skills/$s/SKILL.md" "skills/$s/opencode/agent.md" "skills/$s/codex/prompt.md"; do
            grep -q '^## Existing spec mode' "$f" \
                || fail "$f missing '## Existing spec mode' section"
            grep -q 'git cat-file -e origin/main:<spec-path>' "$f" \
                || fail "$f missing origin/main selected-spec verification"
            grep -q 'If Existing spec mode is active' "$f" \
                || fail "$f missing Phase 3 selected-spec handoff"
        done
    done
}

# Docs amendment presence checks (Phase 10c): every install of autospec that
# has run the docs pipeline must ship the 7 generated artifacts. validate.sh
# fails if any are absent so the lockstep invariant is enforced on every CI run.
#
# Checks:
#   1. docs/USER_MANUAL.md present
#   2. llms.txt present
#   3. docs/.llm-manifest.json present
#   4. skills/autospec-test/SKILL.md carries the Stage 2.5 composition note
check_docs_amendment_presence() {
    info "docs amendment presence: generated artifacts"
    [ -f "docs/USER_MANUAL.md" ] \
        || fail "docs/USER_MANUAL.md: missing — run reverse-engineer.sh + gen-docs-from-spec.mjs to regenerate"
    [ -f "llms.txt" ] \
        || fail "llms.txt: missing — run gen-llms-txt.sh --repo-root . to regenerate"
    [ -f "docs/.llm-manifest.json" ] \
        || fail "docs/.llm-manifest.json: missing — run gen-llm-manifest.mjs to regenerate"
    info "docs amendment presence: autospec-test Stage 2.5 composition note"
    grep -q 'drift.gate\|drift gate\|drift-gate\|check-doc-drift\|doc.*drift\|Stage 2\.5.*drift\|drift.*Stage 2\.5' \
        "skills/autospec-test/SKILL.md" \
        || fail "skills/autospec-test/SKILL.md: missing Stage 2.5 drift-gate composition note (docs amendment §10)"
}

# autospec-test skill structural check (Phase 10): run per-skill validate.sh
# to verify SKILL.md, codex/prompt.md, and structural section presence.
check_autospec_test_skill_present() {
    info "autospec-test skill structural lint"
    local skill_dir="skills/autospec-test"
    local validate_sh="$skill_dir/validate.sh"
    [ -d "$skill_dir" ] || fail "$skill_dir: directory missing"
    [ -f "$skill_dir/SKILL.md" ] || fail "$skill_dir/SKILL.md: required file missing"
    [ -f "$skill_dir/codex/prompt.md" ] || fail "$skill_dir/codex/prompt.md: required file missing"
    [ -f "$validate_sh" ] || fail "$validate_sh: per-skill validate.sh missing"
    bash -n "$validate_sh" || fail "$validate_sh: bash syntax error"
    bash "$validate_sh" >/dev/null 2>&1 \
        || { bash "$validate_sh" >&2; fail "$validate_sh: structural lint failed"; }
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
        check_harness_detection_block "$skill_dir/SKILL.md"
        check_monitor_batch_exit "$skill_dir/SKILL.md"
    done

    info "scanning duo-harness skills under skills/ ..."
    duo_skills="$(discover_duo_skills)"
    for skill_dir in $duo_skills; do
        check_lockstep_duo "$skill_dir"
    done

    check_startup_preflight
    check_stop_mode_section
    check_keyword_routing_section
    check_gap_remediation_section
    check_review_remediation_section
    check_codex_skills_install
    check_shared_script_install

    check_agents_md_subagent_section
    check_autospec_listen_files
    check_examples_dir
    check_governance_headings
    check_existing_spec_mode
    check_lint_issue_helpers
    check_lint_implementation_helpers
    check_phase4_guardian_block_lockstep
    check_phase4_issue_start_summary
    check_phase4_immediate_next_issue_pickup
    check_phase4_adaptive_retry
    check_team_personality_contract
    check_autospec_run_priority_sort_lockstep
    check_autospec_run_regression_review_lockstep
    check_autospec_review_skill_present
    check_autospec_review_tier_a_directives
    check_autospec_test_skill_present
    check_docs_amendment_presence
    check_phase4_tests

    # Top-level installer / uninstaller (introduced in PR #11) — only check syntax
    # if present; absence is OK before that PR lands.
    check_bash_syntax "install.sh"
    check_bash_syntax "uninstall.sh"

    info "OK — all validation checks passed."
}

# tests/phase4/*.sh — exercise the v2-flow Phase 4 implementer prompt
# (structure check, rebase-gate fixtures, migration-replay fixtures).
# tests/docs/*.sh — content checks for operator-facing docs
# (target-repo setup guide, etc.).
# Each script is self-contained and exits 0 on PASS.
check_phase4_tests() {
    info "phase4 tests: tests/phase4/*.sh"
    if [ -d tests/phase4 ]; then
        for t in tests/phase4/*.sh; do
            [ -f "$t" ] || continue
            info "  running: $t"
            bash "$t" >/tmp/validate-phase4.log 2>&1 \
                || { cat /tmp/validate-phase4.log >&2; fail "$t: failed"; }
        done
    fi
    info "docs tests: tests/docs/*.sh"
    if [ -d tests/docs ]; then
        for t in tests/docs/*.sh; do
            [ -f "$t" ] || continue
            info "  running: $t"
            bash "$t" >/tmp/validate-docs.log 2>&1 \
                || { cat /tmp/validate-docs.log >&2; fail "$t: failed"; }
        done
    fi
}

main "$@"
