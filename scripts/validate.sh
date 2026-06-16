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

# ── Fast mode ────────────────────────────────────────────────────────────────
# `--no-bats` / `--fast` skips the bats SUITES (the slow part, ~minutes) while
# still running every structural check (lock-step, goldens, frontmatter, named
# content, file presence) for quick local feedback. CI/pre-merge runs the full
# suite by default.
#
# Implemented by shadowing the `bats` command with a wrapper: every `bats ...`
# call (guarded or not) routes through it. `_REAL_BATS` is detected BEFORE the
# function is defined, so the wrapper preserves today's "skip when bats is not
# installed" behavior even though `command -v bats` now resolves to the function.
RUN_BATS=1
_REAL_BATS=0; command -v bats >/dev/null 2>&1 && _REAL_BATS=1
bats() {
    { [ "$RUN_BATS" = 1 ] && [ "$_REAL_BATS" = 1 ]; } || return 0
    command bats "$@"
}
for _arg in "$@"; do
    case "$_arg" in
        --no-bats|--fast) RUN_BATS=0 ;;
    esac
done
[ "$RUN_BATS" = 1 ] || printf 'validate: fast mode — skipping bats suites (structural checks only)\n' >&2

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
        # Issue #910: the explore/discover build-intent verb-map row must route
        # to /autospec-explore, and the explore-confirm gate literal (emitted by
        # the #909 classifier in scripts/listener-match.sh) must be honored in
        # all three trio files. Keep these byte-identical to the classifier.
        grep -q '/autospec-explore' "$skill_dir/$trio" \
            || fail "autospec-listen: $trio missing explore -> /autospec-explore verb-map row"
        grep -q 'explore-confirm' "$skill_dir/$trio" \
            || fail "autospec-listen: $trio missing 'explore-confirm' gate literal"
        # Issue #954: the imperative `fix` verb-map row must route to the
        # `/autospec` umbrella with NO gate. The `fix` trigger literal is emitted
        # by the #954 classifier branch in scripts/listener-match.sh; keep the
        # trio row in lockstep with it.
        grep -q '| `fix`' "$skill_dir/$trio" \
            || fail "autospec-listen: $trio missing 'fix' -> /autospec verb-map row"
    done
    # The explore-confirm gate literal honored in the trio MUST exactly match the
    # one emitted by the classifier (scripts/listener-match.sh, issue #909).
    grep -q 'explore-confirm' scripts/listener-match.sh \
        || fail "scripts/listener-match.sh missing 'explore-confirm' gate literal (classifier/trio drift)"
    # The `fix` trigger literal honored in the trio MUST exactly match the one
    # emitted by the classifier (scripts/listener-match.sh, issue #954).
    grep -q 'autospec fix imperative' scripts/listener-match.sh \
        || fail "scripts/listener-match.sh missing 'fix' imperative route (classifier/trio drift)"
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

# Enforcement-defaults invariants (autospec-secaudit): the autospec-secaudit
# trio must carry an `## Enforcement defaults` heading in all three trio files
# (SKILL.md, opencode/agent.md, codex/prompt.md) so the concern->gate mapping
# (which dimensions block vs. are advisory) stays in lockstep across harnesses.
# Parallels check_review_remediation_section.
check_enforcement_defaults_section() {
    skill_dir="skills/autospec-secaudit"
    [ -d "$skill_dir" ] || return 0
    info "enforcement-defaults: autospec-secaudit"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Enforcement defaults' "$skill_dir/$trio" \
            || fail "autospec-secaudit: $trio missing '## Enforcement defaults' section"
    done
}

# Self-update invariants (introduced by issue #10): every multi-harness skill
# must document `## Self-update mode` in all three trio files, and its
# install.sh must accept the `--update` flag.
check_self_update() {
    skill_dir="$1"
    name="$(basename "$skill_dir")"
    info "self-update: $name"
    # Transition-safe: accept EITHER the canonical '## Self-update mode' heading
    # OR its <!-- autospec-block:startup-self-update --> marker (D3 sweep form).
    # A file with NEITHER form fails the gate (fail closed).
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        if ! grep -q '^## Self-update mode' "$skill_dir/$trio" \
            && ! grep -q 'autospec-block:startup-self-update' "$skill_dir/$trio"; then
            fail "$name: $trio missing '## Self-update mode' section or autospec-block:startup-self-update marker"
        fi
    done
    grep -q -- '--update' "$skill_dir/install.sh" \
        || fail "$name: install.sh missing --update flag handling"
}

# Startup preflight invariants (introduced by issues #115–#119): every
# multi-harness skill must carry a ## Startup self-update section whose bash
# body is byte-identical to the canonical block (modulo the SKILL_NAME= first
# line).
# Single-source-of-truth (TOKR1-001, issue #1035): the canonical block is read
# from templates/skill-blocks/startup-self-update.md — the same template the
# expander injects — instead of being extracted from skills/autospec/SKILL.md,
# which now carries only the marker (so extraction returned empty -> exit 1).
# Transition-safe (D3): a file carrying <!-- autospec-block:startup-self-update -->
# instead of the full section body is accepted as equivalent; files with NEITHER
# form still fail (fail closed).
check_startup_preflight() {
    extract_block() {
        awk '/^## Startup self-update/{f=1} f && /^```bash/{g=1; next} g && /^```/{g=0; f=0; next} g{print}' "$1" \
            | grep -v '^SKILL_NAME='
    }
    canonical_template="templates/skill-blocks/startup-self-update.md"
    [ -f "$canonical_template" ] \
        || fail "check_startup_preflight: $canonical_template missing (single source of truth)"
    canonical=$(extract_block "$canonical_template")
    [ -n "$canonical" ] || fail "$canonical_template missing ## Startup self-update bash block"
    printf '%s\n' "$canonical" | grep -F 'raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh' >/dev/null \
        || fail "startup preflight must call the curl-safe suite bootstrap.sh"
    printf '%s\n' "$canonical" | grep -F -- '--skill all --harness all --update' >/dev/null \
        || fail "startup preflight must update all skills across all harnesses"
    if printf '%s\n' "$canonical" | grep -F 'raw.githubusercontent.com/berlinguyinca/autospec/main/install.sh' >/dev/null \
        || printf '%s\n' "$canonical" | grep -F 'raw.githubusercontent.com/berlinguyinca/autospec/main/skills/' >/dev/null; then
        fail "startup preflight must not call a raw installer directly"
    fi
    for s in autospec autospec-release autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop autospec-sweep autospec-design autospec-fleet autospec-qa autospec-resume autospec-doc; do
        for f in "skills/$s/SKILL.md" "skills/$s/opencode/agent.md" "skills/$s/codex/prompt.md"; do
            # Transition-safe: if the file carries the marker, skip the byte-diff check
            # (the D3 sweep will expand it; expanded bytes will match the golden).
            if grep -q 'autospec-block:startup-self-update' "$f" 2>/dev/null; then
                continue
            fi
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
    for s in autospec autospec-release autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop autospec-sweep autospec-design autospec-fleet autospec-qa autospec-doc; do
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
    helpers="autospec-stop.sh autospec-usage-limit.sh autospec-watchdog.sh autospec-watchdog.ps1 lint-implementation.sh lint-issue.sh listener-match.sh sizing-check.sh"
    for s in autospec autospec-release autospec-split autospec-define autospec-run autospec-listen autospec-classify autospec-story autospec-stop autospec-sweep autospec-design autospec-fleet autospec-qa autospec-doc; do
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
    # Accept EITHER the literal row OR the harness-adapter block marker (transition-safe: D3b).
    grep -q -E '^\| Subagent model tier ' "$skill_dir/SKILL.md" \
        || grep -q -F '<!-- autospec-block:harness-adapter -->' "$skill_dir/SKILL.md" \
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
        autospec-design)
            expected_a=2; expected_b=0 ;;
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

check_phase1_bounded_context_contract() {
    info "phase1 bounded-context contract: autospec + autospec-define"
    for f in \
        skills/autospec/SKILL.md \
        skills/autospec/codex/prompt.md \
        skills/autospec/opencode/agent.md \
        skills/autospec-define/SKILL.md \
        skills/autospec-define/codex/prompt.md \
        skills/autospec-define/opencode/agent.md
    do
        grep -q 'Phase 1 bounded-context rule' "$f" \
            || fail "$f missing Phase 1 bounded-context rule"
        grep -q 'Do NOT fork, inherit, or compact the full parent conversation' "$f" \
            || fail "$f allows inherited parent conversation in Phase 1"
        grep -q 'fork_context=false' "$f" \
            || fail "$f missing Codex fresh-context directive"
        grep -q 'context window or remote compact failure' "$f" \
            || fail "$f missing compact-overflow fallback directive"
        grep -q 'bounded local read-only `rg`/file-read investigation' "$f" \
            || fail "$f missing bounded local fallback directive"
    done
}

check_autospec_sweep_config_contract() {
    info "autospec-sweep config contract"
    local skill_dir="skills/autospec-sweep"
    [ -d "$skill_dir" ] || fail "$skill_dir: directory missing"
    [ -f "$skill_dir/scripts/wizard.sh" ] || fail "$skill_dir/scripts/wizard.sh: required file missing"
    [ -f "$skill_dir/scripts/run.sh" ] || fail "$skill_dir/scripts/run.sh: required file missing"
    [ -f "$skill_dir/scripts/review.sh" ] || fail "$skill_dir/scripts/review.sh: required file missing"
    [ -f "schemas/autospec-config.schema.json" ] || fail "schemas/autospec-config.schema.json: required file missing"
    bash -n "$skill_dir/scripts/wizard.sh" || fail "$skill_dir/scripts/wizard.sh: bash syntax error"
    bash -n "$skill_dir/scripts/run.sh" || fail "$skill_dir/scripts/run.sh: bash syntax error"
    bash -n "$skill_dir/scripts/review.sh" || fail "$skill_dir/scripts/review.sh: bash syntax error"
    grep -q '.autospec/autospec.yml' "$skill_dir/SKILL.md" \
        || fail "$skill_dir/SKILL.md missing .autospec/autospec.yml contract"
    grep -q 'continuous improvement' "$skill_dir/SKILL.md" \
        || fail "$skill_dir/SKILL.md missing continuous improvement contract"
    for f in skills/autospec/SKILL.md skills/autospec/codex/prompt.md skills/autospec/opencode/agent.md; do
        grep -q '.autospec/autospec.yml' "$f" \
            || fail "$f missing autospec.yml first-run preflight"
        grep -q '/autospec-sweep init' "$f" \
            || fail "$f missing /autospec-sweep init first-run route"
    done
}

check_autospec_fleet_scripts() {
    info "autospec-fleet scripts"
    local skill_dir="skills/autospec-fleet"
    for f in fleet-lib.sh fleet-init.sh fleet-config-lint.sh; do
        [ -f "$skill_dir/scripts/$f" ] || fail "$skill_dir/scripts/$f: required file missing"
        bash -n "$skill_dir/scripts/$f" || fail "$skill_dir/scripts/$f: bash syntax error"
    done
}

# Lockstep check: the literal line '/autospec-fleet gui' must appear in all
# three adapter files for the fleet skill (SKILL.md, codex/prompt.md,
# opencode/agent.md). Introduced by issue #829.
check_fleet_gui_subcommand_lockstep() {
    local needle='/autospec-fleet gui'
    info "fleet-gui subcommand lockstep"
    for f in skills/autospec-fleet/SKILL.md \
              skills/autospec-fleet/codex/prompt.md \
              skills/autospec-fleet/opencode/agent.md; do
        grep -qF "$needle" "$f" \
            || fail "fleet-gui lockstep: '$needle' missing in $f"
    done
}

# Harness detection block validation.
# If a SKILL.md contains a "## Harness detection" heading, verify that the
# section body includes TIER_A, TIER_B, and a "silently" fallback reference.
# If the heading is absent, emit a WARN (not FAIL) to allow incremental migration.
check_harness_detection_block() {
    file="$1"
    name="$(basename "$(dirname "$file")")"
    if ! grep -q '## Harness detection' "$file"; then
        # Migration complete (all skills carry the section); a new skill omitting
        # it is now a regression, not incremental migration — fail closed.
        fail "$name: missing ## Harness detection section (required; defines TIER_A/TIER_B)"
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

# Issue #729: AGENTS.md must declare the canonical Subagent-vs-inline decision
# matrix (9 work-shape rows), and every skill trio's Required-capabilities table
# must carry a `Subagent dispatch policy` row pointing back to it. This keeps
# fan-out vs inline choices uniform across the autospec skill family.
check_agents_md_subagent_matrix() {
    info "AGENTS.md: Subagent vs inline decision matrix + per-skill trio reference"
    [ -f AGENTS.md ] || fail "AGENTS.md missing at repo root"
    grep -q '^## Subagent vs inline decision matrix$' AGENTS.md \
        || fail "AGENTS.md missing '## Subagent vs inline decision matrix' section"
    # The 9 work-shape rows from issue #729 — exact-text anchors so reorderings
    # or rewordings trip the gate.
    for anchor in \
        'Read-only exploration across many files' \
        '2+ independent tasks with disjoint scopes' \
        'Single-purpose long-running work' \
        'Risky / quarantine-worthy work' \
        'Orchestration / decision-making' \
        'One-shot tool calls' \
        'Short edits (1-3 file modifications)' \
        'Routing / control flow' \
        'Multi-area fan-out'
    do
        grep -q "$anchor" AGENTS.md \
            || fail "AGENTS.md decision matrix missing work-shape row: '$anchor'"
    done
    # Every autospec skill trio file must carry the new dispatch-policy row.
    for d in skills/autospec*/; do
        [ -d "$d" ] || continue
        for f in "${d}SKILL.md" "${d}codex/prompt.md" "${d}opencode/agent.md"; do
            [ -f "$f" ] || continue
            grep -q '^## Required capabilities & harness adapter' "$f" || continue
            # Accept EITHER the literal dispatch row OR the harness-adapter-core
            # block marker (transition-safe: D3 split, #1037 — the marker expands
            # to that exact row). Mirrors the harness-adapter marker precedent.
            if grep -q -F '<!-- autospec-block:harness-adapter-core -->' "$f"; then
                continue
            fi
            grep -q '^| Subagent dispatch policy' "$f" \
                || fail "$f: Required-capabilities table missing 'Subagent dispatch policy' row"
            grep -q 'per AGENTS.md decision matrix' "$f" \
                || fail "$f: dispatch-policy row missing 'per AGENTS.md decision matrix' reference"
        done
    done
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

# Implementer-contract named-content invariants (issue #940, spec §D3): the
# curated implementer-contract.md replaces the 70KB SKILL.md in the implementer
# prefix. The contract MUST exist, stay <=24KB (size budget so the cached prefix
# is small), carry exactly one RULE_ID table header (single copy, no dup), and
# must not drift from AGENTS.md's canonical RULE_ID set. The bundler MUST NOT
# inject the autospec-run SKILL.md into the implementer role any more.
check_implementer_contract() {
    info "implementer-contract: skills/autospec-run/prompts/implementer-contract.md"
    local contract="skills/autospec-run/prompts/implementer-contract.md"
    [ -f "$contract" ] \
        || fail "$contract: file missing"

    # Size guard: <=24576 bytes.
    local size
    size="$(wc -c < "$contract" | tr -d ' ')"
    [ "$size" -le 24576 ] \
        || fail "$contract: $size bytes exceeds 24576-byte (24KB) budget"

    # Single RULE_ID table header (no duplicate re-extraction).
    local header_count
    header_count="$(grep -c '^### RULE_ID table' "$contract" || true)"
    [ "$header_count" = "1" ] \
        || fail "$contract: expected exactly 1 '### RULE_ID table' header, found $header_count"

    # Drift gate: every RULE_ID named in AGENTS.md's canonical table must also be
    # present in the contract (curated digest must not silently drop a rule).
    [ -f AGENTS.md ] || fail "AGENTS.md missing at repo root"
    # Extract only the FIRST-column backticked token from each table data row so
    # backticked regex fragments in later columns (e.g. `URL`) are never mistaken
    # for RULE_IDs.
    local agents_rule_ids
    agents_rule_ids="$(awk '
        /^### RULE_ID table/ { in_table=1; next }
        /^### / && in_table { exit }
        /^## / && in_table { exit }
        in_table && /^\| `[A-Z_]+`/ {
            line=$0; sub(/^\| `/,"",line); sub(/`.*/,"",line); print line
        }
    ' AGENTS.md | sort -u)"
    [ -n "$agents_rule_ids" ] || fail "AGENTS.md RULE_ID table yielded no RULE_IDs (parse drift)"
    local rid
    for rid in $agents_rule_ids; do
        grep -q "$rid" "$contract" \
            || fail "$contract: drift — AGENTS.md RULE_ID '$rid' missing from contract"
    done

    # The bundler must inject the contract for the implementer role and must NOT
    # inject the autospec-run SKILL.md for that role any more.
    local bundler="skills/autospec-shared/scripts/bundle-static-context.sh"
    [ -f "$bundler" ] || fail "$bundler: file missing"
    grep -q 'implementer-contract.md' "$bundler" \
        || fail "$bundler: implementer role no longer references implementer-contract.md"
    grep -q 'IMPLEMENTER_CONTRACT' "$bundler" \
        || fail "$bundler: missing IMPLEMENTER_CONTRACT injection variable"

    # Behavioral negative: actual implementer-role output must NOT carry the
    # verbose 'SKILL.md (implementer role)' injection header. This catches a
    # future regression where the bundler emits BOTH the contract and the 70KB
    # SKILL.md (the named-content presence checks above would not).
    local impl_out
    impl_out="$(AUTOSPEC_REPO_ROOT="$REPO_ROOT" \
        AUTOSPEC_MEMORY_DIR=/nonexistent-memory-dir \
        AUTOSPEC_MANIFEST=/nonexistent-manifest.yml \
        bash "$bundler" --role implementer --issue-labels 'ctx:120k' 2>/dev/null)" \
        || fail "$bundler: --role implementer failed to run"
    if printf '%s\n' "$impl_out" | grep -q 'SKILL.md (implementer role)'; then
        fail "$bundler: implementer output still injects the SKILL.md (implementer role) prefix"
    fi
}

# Usage-limit recovery helper invariants: autospec-run's quota recovery path
# depends on this shell-only supervisor being installed and executable.
check_usage_limit_helper() {
    info "usage-limit helper: scripts/autospec-usage-limit.sh"
    [ -f scripts/autospec-usage-limit.sh ] \
        || fail "scripts/autospec-usage-limit.sh: file missing"
    [ -x scripts/autospec-usage-limit.sh ] \
        || fail "scripts/autospec-usage-limit.sh: file not executable"
    bash -n scripts/autospec-usage-limit.sh \
        || fail "scripts/autospec-usage-limit.sh: bash syntax error"
    bash scripts/autospec-usage-limit.sh --help 2>/dev/null | grep -q '^Usage:' \
        || fail "scripts/autospec-usage-limit.sh --help did not print a 'Usage:' line"
}

# Spec supersession contract (issue #635): scripts/resolve-spec-supersession.sh
# must exist, be executable, pass bash -n, and `--help` must print a `Usage:`
# line. Each of autospec-define, autospec-qa, autospec-release must reference
# the resolver in all three trio files (SKILL.md + codex/prompt.md +
# opencode/agent.md) under a `## Spec supersession (recency)` heading so the
# lockstep stays uniform across adapters.
check_supersession_contract() {
    info "spec-supersession helper: scripts/resolve-spec-supersession.sh"
    [ -f scripts/resolve-spec-supersession.sh ] \
        || fail "scripts/resolve-spec-supersession.sh: file missing"
    [ -x scripts/resolve-spec-supersession.sh ] \
        || fail "scripts/resolve-spec-supersession.sh: file not executable"
    bash -n scripts/resolve-spec-supersession.sh \
        || fail "scripts/resolve-spec-supersession.sh: bash syntax error"
    bash scripts/resolve-spec-supersession.sh --help 2>/dev/null | grep -q '^Usage:' \
        || fail "scripts/resolve-spec-supersession.sh --help did not print a 'Usage:' line"
    for s in autospec-define autospec-qa autospec-release; do
        info "spec-supersession lockstep: $s"
        for trio in SKILL.md codex/prompt.md opencode/agent.md; do
            grep -q '^## Spec supersession (recency)' "skills/$s/$trio" \
                || fail "$s: $trio missing '## Spec supersession (recency)' section"
            grep -q 'resolve-spec-supersession\.sh' "skills/$s/$trio" \
                || fail "$s: $trio missing reference to scripts/resolve-spec-supersession.sh"
        done
    done
    [ -f tests/resolve-spec-supersession.bats ] \
        || fail "tests/resolve-spec-supersession.bats: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/resolve-spec-supersession.bats"
        bats tests/resolve-spec-supersession.bats >/tmp/validate-supersession.log 2>&1 \
            || { cat /tmp/validate-supersession.log >&2; fail "tests/resolve-spec-supersession.bats: failed"; }
    fi
}

# Ship-completeness gate (#556/#985): every script a skill surface invokes via
# ${AUTOSPEC_SCRIPTS_DIR} must be shipped by the installers, every $SCRIPT_DIR/lib
# source must ship via copy_runtime_subdirs, and no bare repo-relative invocation
# may remain in a surface file. Run tests/ship-completeness.bats as part of the
# lock-step gate so install-drift fails closed HERE, not silently in a target repo.
check_ship_completeness() {
    [ -f tests/ship-completeness.bats ] \
        || fail "tests/ship-completeness.bats: missing (ship-completeness gate)"
    if command -v bats >/dev/null 2>&1; then
        info "ship-completeness gate: tests/ship-completeness.bats"
        bats tests/ship-completeness.bats >/tmp/validate-ship-completeness.log 2>&1 \
            || { cat /tmp/validate-ship-completeness.log >&2; fail "tests/ship-completeness.bats: failed"; }
    fi
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

# autospec-run queue-drain contract: `reasoning:deep` may shorten one monitor
# batch for fresh context, but it must not end the user-invoked run.
check_autospec_run_continuation_contract() {
    info "autospec-run continuation contract: BATCH_COMPLETE relaunches until ALL_DONE"
    for f in \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
    do
        [ -f "$f" ] || fail "$f: required file missing"
        grep -F 'BATCH_COMPLETE is a continuation signal, not a terminal state' "$f" >/dev/null \
            || fail "$f missing BATCH_COMPLETE continuation contract"
        grep -F 'reasoning:deep may reduce a single monitor batch to one issue' "$f" >/dev/null \
            || fail "$f missing reasoning:deep single-monitor-batch wording"
        grep -F 'the orchestrator MUST relaunch automatically until ALL_DONE' "$f" >/dev/null \
            || fail "$f missing automatic relaunch-until-ALL_DONE directive"
        grep -F 'Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE' "$f" >/dev/null \
            || fail "$f missing no-manual-rerun directive"
    done
}

# Phase 4 single-agent absorbed-discipline (issue #648): the autospec-run trio
# must (1) carry the canonical constraint sentence stating that subagents spawned
# by background `Agent` calls do NOT inherit the `Agent` tool, (2) reference the
# single-agent absorbed-discipline rewrite, (3) document that Phase 4 implementers
# must be top-level agents launched directly by the main session orchestrator,
# and (4) NOT contain the legacy "process(ISSUE)   # foreground subagent" or
# "dispatches a **foreground subagent**" patterns that previously misled
# operators into nested-dispatch. The phase4-implementer.md prompt must also
# state the single-agent assumption explicitly.
check_phase4_single_agent_discipline() {
    info "phase4 single-agent absorbed-discipline: autospec-run + autospec trios"
    for f in \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md \
        skills/autospec/SKILL.md \
        skills/autospec/codex/prompt.md \
        skills/autospec/opencode/agent.md
    do
        [ -f "$f" ] || fail "$f: required file missing"
        grep -F 'Subagents spawned by background `Agent` calls do NOT inherit the `Agent` tool' "$f" >/dev/null \
            || fail "$f missing canonical subagent-Agent-inheritance constraint sentence"
        grep -F 'single-agent absorbed-discipline' "$f" >/dev/null \
            || fail "$f missing 'single-agent absorbed-discipline' reference"
        grep -F 'top-level agents launched directly by the main session orchestrator' "$f" >/dev/null \
            || fail "$f missing 'top-level agents launched directly by the main session orchestrator' sentence"
        if grep -F 'process(ISSUE)   # foreground subagent' "$f" >/dev/null 2>&1; then
            fail "$f still contains legacy 'process(ISSUE)   # foreground subagent' nested-dispatch pattern"
        fi
        if grep -F 'dispatches a **foreground subagent**' "$f" >/dev/null 2>&1; then
            fail "$f still contains legacy 'dispatches a **foreground subagent**' nested-dispatch pattern"
        fi
    done
    impl="skills/autospec-run/prompts/phase4-implementer.md"
    [ -f "$impl" ] || fail "$impl: required file missing"
    grep -F 'single-agent' "$impl" >/dev/null \
        || fail "$impl missing 'single-agent' assumption in prompt body"
    grep -F 'Subagents spawned by background `Agent` calls do NOT inherit the `Agent` tool' "$impl" >/dev/null \
        || fail "$impl missing canonical subagent-Agent-inheritance constraint sentence"
    bats_file="tests/phase4/test_single_agent_discipline.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-single-agent-discipline.log 2>&1 \
            || { cat /tmp/validate-single-agent-discipline.log >&2; fail "$bats_file: failed"; }
    fi
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

# Phase 4 full-suite verification: autospec must never merge a PR just because
# review and required CI are green. Every Phase 4 adapter must require the
# target repo's full local validation/test suite before LGTM review and again
# after branch update/rebase immediately before admin-merge. Failures must
# return to the fix/recommit/retry loop; no merge is allowed while the suite
# fails.
check_phase4_full_test_suite_gate() {
    info "phase4 full test suite gate: autospec + autospec-run trio"
    for f in \
        skills/autospec/SKILL.md \
        skills/autospec/codex/prompt.md \
        skills/autospec/opencode/agent.md \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
    do
        grep -q 'Full test suite gate' "$f" \
            || fail "$f missing Full test suite gate section"
        grep -q 'AUTOSPEC_FULL_TEST_COMMAND' "$f" \
            || fail "$f missing AUTOSPEC_FULL_TEST_COMMAND override"
        grep -q 'scripts/validate.sh' "$f" \
            || fail "$f missing scripts/validate.sh fallback command"
        grep -q 'If the full suite fails, fix the failure, recommit, rerun the full suite, and repeat' "$f" \
            || fail "$f missing fix/recommit/rerun failure loop"
        grep -q 'Do NOT dispatch LGTM review' "$f" \
            || fail "$f missing pre-review block on failing full suite"
        grep -q 'Do NOT run `gh pr merge`' "$f" \
            || fail "$f missing pre-merge block on failing full suite"
        grep -q 'Record the exact full-suite command and passing output summary' "$f" \
            || fail "$f missing verification evidence requirement"
    done
}

# Phase 4 cost-epic parity (issue #971): the cost-efficiency epic (#937) D1
# (per-issue token reporting) and D2 (batch=1 / fresh-subagent-per-issue) were
# applied to the autospec-run trio but silently missed the autospec trio's
# embedded Phase 4 monitor loop, and the existing phase4 lock-step gates
# (guardian / issue-start / next-pickup / test-suite) did NOT cover the
# batch-default, token-report slot, or absorbed-discipline prose — so the
# divergence passed CI. This gate ties those three surfaces between BOTH run
# trios so the divergence CLASS fails CI going forward. Each assertion below
# must PASS after parity and provably FAIL if either trio drifts back.
check_phase4_cost_epic_parity_lockstep() {
    info "phase4 cost-epic parity (D1 token-report + D2 batch=1): autospec + autospec-run trios"
    for s in autospec autospec-run; do
        for f in "skills/$s/SKILL.md" "skills/$s/codex/prompt.md" "skills/$s/opencode/agent.md"; do
            [ -f "$f" ] || fail "$f: required file missing"
            # D2: batch default must be 1, never the legacy 3, anywhere a
            # batch-size default or prose is stated.
            if grep -F 'AUTOSPEC_BATCH_SIZE:-3' "$f" >/dev/null 2>&1; then
                fail "$f still defaults AUTOSPEC_BATCH_SIZE to 3 (D2 requires :-1)"
            fi
            if grep -Eq 'AUTOSPEC_BATCH_SIZE` issues \(default:? 3\)' "$f"; then
                fail "$f still states batch 'default 3' prose (D2 requires 1)"
            fi
            grep -F 'AUTOSPEC_BATCH_SIZE:-1' "$f" >/dev/null \
                || fail "$f missing AUTOSPEC_BATCH_SIZE:-1 default (D2 batch=1)"
            # D2: fresh-subagent-per-issue prose (orchestrator never implements
            # in its own context; default is 1; reasoning:deep force-to-1).
            grep -F 'Fresh-subagent-per-issue (canonical Phase 4 path, formerly single-agent absorbed-discipline)' "$f" >/dev/null \
                || fail "$f missing fresh-subagent-per-issue canonical prose (D2)"
            grep -F 'The orchestrator NEVER implements in its own context' "$f" >/dev/null \
                || fail "$f missing 'orchestrator NEVER implements in its own context' (D2)"
            grep -F 'the default is 1 (one issue per subagent)' "$f" >/dev/null \
                || fail "$f missing 'default is 1 (one issue per subagent)' (D2)"
            grep -F 'reasoning:deep` force-to-1 rule is retained' "$f" >/dev/null \
                || fail "$f missing reasoning:deep force-to-1 retention (D2)"
            # D1: per-issue token-report step at the pinned slot — after the
            # admin-merge SUCCESS step (8) and before the FAILURE step (9) /
            # batch-done. Enforce both the markers and their pinned position.
            grep -F '<!-- token-report:begin -->' "$f" >/dev/null \
                || fail "$f missing token-report:begin marker (D1)"
            grep -F '<!-- token-report:end -->' "$f" >/dev/null \
                || fail "$f missing token-report:end marker (D1)"
            grep -F 'post-token-report.sh' "$f" >/dev/null \
                || fail "$f missing post-token-report.sh invocation (D1)"
            slot="$(awk '
                /(^|> )8\. SUCCESS/             { after_success=1 }
                /(^|> )9\. FAILURE/             { after_success=0 }
                after_success && /token-report:begin/ { print "FOUND" }
            ' "$f")"
            [ "$slot" = "FOUND" ] \
                || fail "$f: token-report:begin not at pinned slot (must follow admin-merge SUCCESS step 8, precede FAILURE step 9 / batch-done) (D1)"
        done
    done
    info "phase4 cost-epic parity: both trios carry batch=1 + fresh-subagent prose + pinned token-report"
}

# Docs-drift-gate regeneration conditional parity (issue #955): the D2b gate
# conditional wrapping the regeneration path must be present in BOTH run trios
# (autospec + autospec-run, 6 files total). Detection, labels, and the skip log
# line must be present. Fails CI if either trio drifts back to unconditional regen.
check_docs_drift_gate_regen_conditional_parity() {
    info "docs-drift-gate regen conditional parity (D2b): autospec + autospec-run trios (6 files)"
    for s in autospec autospec-run; do
        for f in "skills/$s/SKILL.md" "skills/$s/codex/prompt.md" "skills/$s/opencode/agent.md"; do
            [ -f "$f" ] || fail "$f: required file missing"
            # Gate conditional: _REGEN resolver must be present.
            grep -q '_REGEN' "$f" \
                || fail "$f: missing _REGEN gate variable (D2b auto_regenerate conditional)"
            # Resolver function must be referenced.
            grep -q 'resolveAutoRegenerate' "$f" \
                || fail "$f: missing resolveAutoRegenerate import in gate block (D2b)"
            # OFF-path log line must be present.
            grep -qF 'docs: regeneration skipped (auto_regenerate=false)' "$f" \
                || fail "$f: missing OFF-path log 'docs: regeneration skipped (auto_regenerate=false)' (D2b)"
            # docs-drift-gate markers must still bracket the section.
            grep -qF '<!-- docs-drift-gate:begin -->' "$f" \
                || fail "$f: missing docs-drift-gate:begin marker (D2b)"
            grep -qF '<!-- docs-drift-gate:end -->' "$f" \
                || fail "$f: missing docs-drift-gate:end marker (D2b)"
            # Regeneration path (orchestrator) must still be present inside the gate.
            grep -q 'doc-orchestrator.mjs' "$f" \
                || fail "$f: missing doc-orchestrator.mjs inside gate (D2b regenerate path must be preserved)"
        done
    done
    info "docs-drift-gate regen conditional parity: all 6 trio files carry D2b gate"
}

# Worktree PR-aware ladder + assert-gate parity (issue #961, spec §D3): the
# run-trio `process(ISSUE)` step 1 must carry the 3-state recovery ladder
# (open-pr/branch-only/fresh) wired through `worktree-guard.sh resolve-branch`,
# plus the mandatory `worktree-guard.sh assert` gate (MUST exit 0 before any
# edit; non-zero → restore auto-implement + stop) and the prose hard rules
# (no primary-checkout mutation; cleanup = `git worktree remove` + prune). Both
# run trios (6 files) AND phase4-implementer.md must agree. Fails CI if either
# trio drifts back to the unconditional `git worktree add` path.
check_worktree_ladder_assert_parity() {
    info "worktree ladder + assert parity (D3): autospec + autospec-run trios (6 files) + phase4-implementer.md"
    local phase4="skills/autospec-run/prompts/phase4-implementer.md"
    local targets=""
    for s in autospec autospec-run; do
        targets="$targets skills/$s/SKILL.md skills/$s/codex/prompt.md skills/$s/opencode/agent.md"
    done
    targets="$targets $phase4"
    for f in $targets; do
        [ -f "$f" ] || fail "$f: required file missing"
        grep -q 'worktree-guard.sh resolve-branch' "$f" \
            || fail "$f: missing worktree-guard.sh resolve-branch (D3 ladder step 1)"
        grep -q 'open-pr' "$f"     || fail "$f: missing open-pr ladder state (D3)"
        grep -q 'branch-only' "$f" || fail "$f: missing branch-only ladder state (D3)"
        grep -q '\bfresh\b' "$f"   || fail "$f: missing fresh ladder state (D3)"
        grep -qi 'skip implementation' "$f" \
            || fail "$f: open-pr path must skip implementation (D3 #886 recovery)"
        grep -q 'adopt' "$f" \
            || fail "$f: branch-only path must adopt the branch (D3 #917 recovery)"
        grep -q 'worktree-guard.sh assert' "$f" \
            || fail "$f: missing worktree-guard.sh assert gate (D3 step 2)"
        grep -q 'MUST exit 0' "$f" \
            || fail "$f: assert gate must require exit 0 before any edit (D3)"
        grep -qi 'primary checkout' "$f" \
            || fail "$f: missing primary-checkout hard rule (D3)"
        grep -q 'git worktree remove' "$f" \
            || fail "$f: missing 'git worktree remove' cleanup rule (D3)"
        grep -q 'git worktree prune' "$f" \
            || fail "$f: missing 'git worktree prune' cleanup rule (D3)"
    done
    # Extracted ladder bash from each SKILL.md must be well-formed.
    for f in skills/autospec/SKILL.md skills/autospec-run/SKILL.md; do
        local block
        block="$(awk '
            /worktree-ladder:begin/{cap=1; next}
            /worktree-ladder:end/{cap=0}
            cap{ sub(/^> ?/, ""); print }
        ' "$f")"
        [ -n "$block" ] \
            || fail "$f: missing worktree-ladder:begin/end sentinel block (D3)"
        printf '%s\n' "$block" | bash -n - \
            || fail "$f: extracted worktree-ladder block fails bash -n (D3)"
    done
    info "worktree ladder + assert parity: all 6 trio files + phase4-implementer.md carry D3 ladder/assert"
}

# Team-personality contract: autospec should infer the right solving team from
# the request, repository evidence, memories, and prior specs. If confidence is
# low, it must ask explicitly with five concrete starter combinations, carry
# the choice into the design spec, include it in child issues, and teach Phase 4
# implementers to honor it as the execution lens.
check_team_personality_selection_contract() {
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
        grep -q 'Critical improvement\|critical improvement' "$f" \
            || fail "$f missing critical improvement self-question checkpoint"
    done
}

check_team_personality_issue_template_contract() {
    for f in "skills/autospec/SKILL.md" "skills/autospec-define/SKILL.md" "skills/autospec-split/SKILL.md"; do
        grep -q -- '- \*\*Team personality\*\*' "$f" \
            || fail "$f missing Team personality child issue section"
        grep -q -- '- \*\*Review counter-team\*\*' "$f" \
            || fail "$f missing Review counter-team child issue section"
        grep -q 'body should already contain `## Files to read first`, `## Implementation scope`, `## Team personality`, and `## Review counter-team`' "$f" \
            || fail "$f missing Phase 3.5 team-lens template gate"
    done
}

check_team_personality_phase4_and_docs_contract() {
    for f in "skills/autospec/SKILL.md" "skills/autospec-run/SKILL.md"; do
        grep -q '## Team personality as execution lens' "$f" \
            || fail "$f missing Phase 4 team personality execution lens"
        grep -q '## Review counter-team as review lens' "$f" \
            || fail "$f missing Phase 4 review counter-team lens"
        grep -q 'Critical self-question before LGTM' "$f" \
            || fail "$f missing Phase 4 critical self-question before LGTM"
    done
    grep -q 'team_personality' scripts/gen-issue-skeleton.sh \
        || fail "scripts/gen-issue-skeleton.sh missing team_personality input"
    grep -q 'review_counter_team' scripts/gen-issue-skeleton.sh \
        || fail "scripts/gen-issue-skeleton.sh missing review_counter_team input"
    grep -q -- '--issue-body' scripts/gen-reviewer-prompt.sh \
        || fail "scripts/gen-reviewer-prompt.sh missing --issue-body support"
    grep -q -- '--issue-body "/tmp/issue-<ISSUE>-body.md"' skills/autospec-run/SKILL.md \
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

check_team_personality_contract() {
    info "team personality contract: autospec / autospec-define / autospec-run"
    check_team_personality_selection_contract
    check_team_personality_issue_template_contract
    check_team_personality_phase4_and_docs_contract
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

check_autospec_stl_design_guardrails() {
    info "autospec STL/CAD design guardrails"
    for f in \
        "skills/autospec/SKILL.md" \
        "skills/autospec/opencode/agent.md" \
        "skills/autospec/codex/prompt.md"
    do
        grep -q '^## Physical STL / CAD design guardrails' "$f" \
            || fail "$f missing physical STL/CAD design guardrails section"
        grep -q 'at least 5 mm of free working clearance' "$f" \
            || fail "$f missing 5 mm fitting/tool clearance rule"
        grep -q 'at least 5 mm of continuous plastic margin' "$f" \
            || fail "$f missing 5 mm gasket plastic margin rule"
        grep -q 'relief valves, vents, restriction' "$f" \
            || fail "$f missing low-flow vacuum no-restrictor rule"
        grep -q 'gas/fluid/vacuum/dust-flow simulation' "$f" \
            || fail "$f missing sealed/flowing simulation rule"
        grep -q 'visual QA from at least 16 angles' "$f" \
            || fail "$f missing 16-angle visual QA rule"
        grep -q 'Regenerate from a clean build directory' "$f" \
            || fail "$f missing clean-build regeneration rule"
    done
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

# Tier right-sizing (issue #941, spec §D4): the second Tier-A regression
# meta-review dispatch is FOLDED into the single fused reviewer brief. The
# run-trio variants must (a) NOT carry a second `TIER_A` meta-review dispatch,
# (b) keep the regression gap-check + reviewer-lessons write-path inside the
# single reviewer brief, and (c) document the `AUTOSPEC_REVIEWER_TIER` hatch.
check_autospec_run_regression_review_lockstep() {
    info "autospec-run regression review folded (D4) lockstep"
    for variant in \
        "skills/autospec-run/SKILL.md" \
        "skills/autospec-run/opencode/agent.md" \
        "skills/autospec-run/codex/prompt.md"; do
        ! grep -q 'dispatch a second `TIER_A` subagent' "$variant" \
            || fail "second TIER_A regression meta-review dispatch still present in $variant (should be folded into reviewer brief)"
        grep -q 'reviewer-lessons.md' "$variant" \
            || fail "reviewer-lessons write-path missing in $variant"
        grep -qi 'would the reviewer have caught the original gap' "$variant" \
            || fail "folded regression gap-check missing in $variant"
        grep -q 'AUTOSPEC_REVIEWER_TIER' "$variant" \
            || fail "AUTOSPEC_REVIEWER_TIER env hatch missing in $variant"
        grep -q '`TIER_B` for ALL issues' "$variant" \
            || fail "reviewer Tier-B-for-all-issues directive missing in $variant"
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
            grep -qE 'git cat-file -e (origin/main|<base>):<spec-path>' "$f" \
                || fail "$f missing selected-spec verification (git cat-file -e <base>|origin/main:<spec-path>)"
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

# autospec-playwright skill structural check (issue #999): run per-skill validate.sh
# to verify trio lockstep, leading blank line in codex/prompt.md, and required sections.
check_autospec_playwright_skill_present() {
    info "autospec-playwright skill structural lint"
    local skill_dir="skills/autospec-playwright"
    local validate_sh="$skill_dir/validate.sh"
    [ -d "$skill_dir" ] || fail "$skill_dir: directory missing"
    [ -f "$skill_dir/SKILL.md" ] || fail "$skill_dir/SKILL.md: required file missing"
    [ -f "$skill_dir/codex/prompt.md" ] || fail "$skill_dir/codex/prompt.md: required file missing"
    [ -f "$validate_sh" ] || fail "$validate_sh: per-skill validate.sh missing"
    bash -n "$validate_sh" || fail "$validate_sh: bash syntax error"
    bash "$validate_sh" >/dev/null 2>&1 \
        || { bash "$validate_sh" >&2; fail "$validate_sh: structural lint failed"; }
}

check_autospec_qa_contract() {
    info "autospec-qa contract: no-mock smoke blocker handling"
    local skill_file="skills/autospec-qa/SKILL.md"
    [ -f "$skill_file" ] || fail "$skill_file: required file missing"

    grep -q '^## No-mock deployed smoke blocker handling' "$skill_file" \
        || fail "$skill_file: missing no-mock smoke blocker handling section"
    grep -q 'must be `PARTIAL`' "$skill_file" \
        || fail "$skill_file: missing PARTIAL verdict requirement for untested deployed smoke"
    grep -q 'environment setup issue/task' "$skill_file" \
        || fail "$skill_file: missing environment setup artifact requirement"
    grep -q 'operator-blocker line with the exact missing configuration' "$skill_file" \
        || fail "$skill_file: missing exact operator blocker requirement"
    grep -q '^## Presentational control classification' "$skill_file" \
        || fail "$skill_file: missing presentational control classification section"
    grep -q '^## User input element intent prompt' "$skill_file" \
        || fail "$skill_file: missing user input element intent prompt section"
    grep -q '^## Live backend blocker triage prompt' "$skill_file" \
        || fail "$skill_file: missing live backend blocker triage prompt section"
    grep -q 'Never print secret values' "$skill_file" \
        || fail "$skill_file: missing secret redaction requirement for live backend triage"
    grep -q 'route-specific worker/handler failure' "$skill_file" \
        || fail "$skill_file: missing route-specific worker failure classification"
    grep -q '^## Proof artifact requirements' "$skill_file" \
        || fail "$skill_file: missing proof artifact requirements section"
    grep -q '^## Generated app reliability contract prompt' "$skill_file" \
        || fail "$skill_file: missing generated app reliability contract prompt"
    grep -q '^## Control intent ledger prompt' "$skill_file" \
        || fail "$skill_file: missing control intent ledger prompt"
    grep -q '^## Duplicate-code guardian prompt' "$skill_file" \
        || fail "$skill_file: missing duplicate-code guardian prompt"
    grep -q '^## Mutation and breakage proof prompt' "$skill_file" \
        || fail "$skill_file: missing mutation and breakage proof prompt"
    grep -q '^## Post-merge deployed canary prompt' "$skill_file" \
        || fail "$skill_file: missing post-merge deployed canary prompt"
    grep -q '^## No-mock minimum coverage prompt' "$skill_file" \
        || fail "$skill_file: missing no-mock minimum coverage prompt"
    grep -q '^## New code intent gate' "$skill_file" \
        || fail "$skill_file: missing new code intent gate"
    grep -q '.autospec/proof-matrix.json' "$skill_file" \
        || fail "$skill_file: missing proof matrix artifact path"
    grep -q '.autospec/reliability.yml' "$skill_file" \
        || fail "$skill_file: missing reliability contract artifact path"
    grep -q '.autospec/control-intent-ledger.json' "$skill_file" \
        || fail "$skill_file: missing control intent ledger artifact path"
    grep -q '.autospec/mutation-proof.json' "$skill_file" \
        || fail "$skill_file: missing mutation proof artifact path"
    grep -q '.autospec/canary-results.json' "$skill_file" \
        || fail "$skill_file: missing canary results artifact path"
    for schema in \
        schemas/autospec-proof-matrix.schema.json \
        schemas/autospec-reliability.schema.json \
        schemas/autospec-control-intent-ledger.schema.json \
        schemas/autospec-mutation-proof.schema.json \
        schemas/autospec-canary-results.schema.json; do
        [ -f "$schema" ] || fail "$schema: QA artifact schema missing"
    done
    [ -x "scripts/validate-qa-artifacts.sh" ] \
        || fail "scripts/validate-qa-artifacts.sh: missing or not executable"
    bash -n "scripts/validate-qa-artifacts.sh" \
        || fail "scripts/validate-qa-artifacts.sh: bash syntax error"
    grep -q '^## Artifact freshness gate' "$skill_file" \
        || fail "$skill_file: missing artifact freshness gate"
    grep -q '^## Evidence provenance gate' "$skill_file" \
        || fail "$skill_file: missing evidence provenance gate"
    grep -q '^## Console and network error gate' "$skill_file" \
        || fail "$skill_file: missing console and network error gate"
    grep -q '^## Spec contradiction detector' "$skill_file" \
        || fail "$skill_file: missing spec contradiction detector"
    grep -q '^## Data lifecycle proof' "$skill_file" \
        || fail "$skill_file: missing data lifecycle proof"
    grep -q '^## Observability contract' "$skill_file" \
        || fail "$skill_file: missing observability contract"
    grep -q '^## Flake quarantine rule' "$skill_file" \
        || fail "$skill_file: missing flake quarantine rule"
    grep -q '^## Reliability exhaustion loop' "$skill_file" \
        || fail "$skill_file: missing reliability exhaustion loop"
    grep -q '^## Legacy removal and spec cleanup gate' "$skill_file" \
        || fail "$skill_file: missing legacy removal and spec cleanup gate"
    grep -q 'Do not populate deprecated caches' "$skill_file" \
        || fail "$skill_file: missing deprecated cache/bucket anti-revival rule"
    grep -q 'deprecated_surfaces' "schemas/autospec-reliability.schema.json" \
        || fail "schemas/autospec-reliability.schema.json: missing deprecated_surfaces contract"
}

check_autospec_sweep_area_contract() {
    info "autospec-sweep area dispatch contract"
    local areas_dir="skills/autospec-sweep/areas"
    [ -d "$areas_dir" ] || fail "$areas_dir: required directory missing"
    for a in spec-vs-code-drift docs-drift code-health dependency-health; do
        [ -f "$areas_dir/$a.md" ] || fail "$areas_dir/$a.md: required area file missing"
    done
    local dispatcher="scripts/sweep-area-dispatch.sh"
    [ -f "$dispatcher" ] || fail "$dispatcher: required dispatcher missing"
    bash -n "$dispatcher" || fail "$dispatcher: bash syntax error"
    local dep_researcher="scripts/explore-research/dependency-health.sh"
    [ -f "$dep_researcher" ] || fail "$dep_researcher: required researcher missing"
    bash -n "$dep_researcher" || fail "$dep_researcher: bash syntax error"
    # Lockstep: the "## Parallel area dispatch" section must appear in all 3
    # trio adapters with the same area list.
    for f in skills/autospec-sweep/SKILL.md \
             skills/autospec-sweep/codex/prompt.md \
             skills/autospec-sweep/opencode/agent.md; do
        grep -q '^## Parallel area dispatch' "$f" \
            || fail "$f: missing '## Parallel area dispatch' section"
        for a in spec-vs-code-drift docs-drift code-health dependency-health; do
            grep -q "$a" "$f" \
                || fail "$f: missing area reference '$a'"
        done
        grep -q 'sweep-area-dispatch.sh' "$f" \
            || fail "$f: missing reference to scripts/sweep-area-dispatch.sh"
    done
    local bats_file="tests/sweep/test_sweep_area_dispatch.bats"
    [ -f "$bats_file" ] || fail "$bats_file: required bats file missing"
    info "  running: $bats_file"
    bats "$bats_file" >/tmp/validate-sweep-area.log 2>&1 \
        || { cat /tmp/validate-sweep-area.log >&2; fail "$bats_file: failed"; }
}

check_autospec_release_contract() {
    info "autospec-release contract: release readiness wrapper"
    local skill_file="skills/autospec-release/SKILL.md"
    [ -f "$skill_file" ] || fail "$skill_file: required file missing"
    [ -f "skills/autospec-release/README.md" ] || fail "skills/autospec-release/README.md: required file missing"

    grep -q '^## Release pipeline' "$skill_file" \
        || fail "$skill_file: missing release pipeline section"
    grep -q '/autospec-sweep' "$skill_file" \
        || fail "$skill_file: missing autospec-sweep stage"
    grep -q '/autospec-review' "$skill_file" \
        || fail "$skill_file: missing autospec-review stage"
    grep -q '/autospec-run' "$skill_file" \
        || fail "$skill_file: missing autospec-run stage"
    grep -q '/autospec-test' "$skill_file" \
        || fail "$skill_file: missing autospec-test stage"
    grep -q '/autospec-qa' "$skill_file" \
        || fail "$skill_file: missing autospec-qa stage"
    grep -q 'validate-qa-artifacts.sh' "$skill_file" \
        || fail "$skill_file: missing QA artifact validation helper"
    grep -q '^## Legacy cleanup prompt' "$skill_file" \
        || fail "$skill_file: missing legacy cleanup prompt"
    grep -q 'PASS`, `PARTIAL`, or `FAIL`' "$skill_file" \
        || fail "$skill_file: missing explicit release verdict vocabulary"
    grep -q 'autospec-release' README.md \
        || fail "README.md: missing autospec-release getting-started entry"
    grep -q 'Getting Started' README.md \
        || fail "README.md: missing getting-started skill guide"
}

check_release_verdict_script() {
    info "release verdict computation: scripts/compute-release-verdict.sh"
    local script="scripts/compute-release-verdict.sh"
    local bats="tests/compute-release-verdict.bats"
    [ -f "$script" ] || fail "$script: required file missing"
    [ -x "$script" ] || fail "$script: must be executable"
    [ -f "$bats" ] || fail "$bats: required file missing"
    bash -n "$script" || fail "$script: bash syntax error"
    # Verify all three release adapters reference the script so the deterministic
    # verdict is the canonical computation rather than re-derived prose.
    for f in skills/autospec-release/SKILL.md \
             skills/autospec-release/codex/prompt.md \
             skills/autospec-release/opencode/agent.md; do
        grep -q 'compute-release-verdict.sh' "$f" \
            || fail "$f: missing reference to scripts/compute-release-verdict.sh"
    done
    info "  running: $bats"
    bats "$bats" >/tmp/validate-verdict.log 2>&1 \
        || { cat /tmp/validate-verdict.log >&2; fail "$bats: failed"; }
}

check_qa_verdict_contract() {
    info "autospec-qa verdict artifact contract"
    for f in skills/autospec-qa/SKILL.md \
             skills/autospec-qa/codex/prompt.md \
             skills/autospec-qa/opencode/agent.md; do
        grep -q '.autospec/qa-verdict.json' "$f" \
            || fail "$f: missing reference to .autospec/qa-verdict.json"
        grep -q 'live_app_proof' "$f" \
            || fail "$f: missing live_app_proof field in verdict schema"
        # Category enum must include the high-priority release-blocking categories
        # added in PRs #634 (outsourced) and earlier (benchmark overfit).
        grep -q 'outsourced_implementation' "$f" \
            || fail "$f: missing outsourced_implementation in category enum"
        grep -q 'benchmark_overfit' "$f" \
            || fail "$f: missing benchmark_overfit in category enum"
    done
    # Release iteration loop must have a remediation branch for every
    # release_blocking category enumerated by QA.
    for f in skills/autospec-release/SKILL.md \
             skills/autospec-release/codex/prompt.md \
             skills/autospec-release/opencode/agent.md; do
        grep -q 'outsourced_implementation' "$f" \
            || fail "$f: missing outsourced_implementation remediation branch"
    done
}

# Brute-force string-heuristics RULE_ID invariants (issue #637): both
# STRING_MATCH_DOMAIN_LOGIC and REPEATED_STRUCTURE_AS_CODE must appear in
# all 6 adapter files (autospec-run trio + autospec-qa trio) so the
# PR-time guardian list and the QA sweep section stay in lockstep with
# AGENTS.md.
check_brute_force_rule_ids() {
    info "brute-force RULE_IDs: autospec-run + autospec-qa trios"
    for rid in STRING_MATCH_DOMAIN_LOGIC REPEATED_STRUCTURE_AS_CODE; do
        grep -q "$rid" AGENTS.md \
            || fail "AGENTS.md missing $rid in RULE_ID table"
        for trio in skills/autospec-run/SKILL.md skills/autospec-run/codex/prompt.md skills/autospec-run/opencode/agent.md \
                    skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
            grep -q "$rid" "$trio" \
                || fail "$trio missing $rid (issue #637 lockstep)"
        done
    done
    # The QA sweep section must appear in all 3 autospec-qa adapter files.
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Brute-force string heuristics sweep' "$trio" \
            || fail "$trio missing '## Brute-force string heuristics sweep' section (issue #637)"
    done
    # The sweep script must exist and be executable.
    [ -x scripts/qa-brute-force-sweep.sh ] \
        || fail "scripts/qa-brute-force-sweep.sh missing or not executable (issue #637)"
}

# Dogfood detectors gate (issue #641): run every heuristic detector that
# autospec ships against this repo and assert findings match the per-detector
# allowlist. Also enforces lockstep across the 3 autospec-qa adapter files —
# the "## Dogfood detectors driver" section and the scripts/dogfood-detectors.sh
# reference must appear in SKILL.md, codex/prompt.md, and opencode/agent.md.
check_dogfood_detectors() {
    info "dogfood detectors: scripts/dogfood-detectors.sh"
    [ -f scripts/dogfood-detectors.sh ] \
        || fail "scripts/dogfood-detectors.sh: file missing"
    [ -x scripts/dogfood-detectors.sh ] \
        || fail "scripts/dogfood-detectors.sh: file not executable"
    bash -n scripts/dogfood-detectors.sh \
        || fail "scripts/dogfood-detectors.sh: bash syntax error"

    info "dogfood detectors: autospec-qa adapter lockstep"
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Dogfood detectors driver' "$trio" \
            || fail "$trio missing '## Dogfood detectors driver' section (issue #641)"
        grep -q 'dogfood-detectors\.sh' "$trio" \
            || fail "$trio missing reference to dogfood-detectors.sh (issue #641)"
    done

    info "dogfood detectors: adapter layer (issue #646)"
    [ -f .autospec/dogfood.yml ] \
        || fail ".autospec/dogfood.yml: missing (issue #646 adapter registry)"
    grep -q '^adapters:' .autospec/dogfood.yml \
        || fail ".autospec/dogfood.yml: missing 'adapters:' schema (issue #646)"
    # The adapter scripts remain available for ad-hoc local runs even when
    # they are intentionally unregistered in dogfood.yml (PR-time tools
    # without a diff context cannot anchor a CI gate — see dogfood.yml
    # rationale comment). Presence + executability + syntax are still
    # checked; registration is no longer required.
    for adapter in scripts/dogfood-adapter-doc-drift.sh scripts/dogfood-adapter-lint.sh; do
        [ -f "$adapter" ] \
            || fail "$adapter: required adapter missing (issue #646)"
        [ -x "$adapter" ] \
            || fail "$adapter: adapter not executable (issue #646)"
        bash -n "$adapter" \
            || fail "$adapter: bash syntax error (issue #646)"
    done

    info "dogfood detectors: running driver against this repo"
    bash scripts/dogfood-detectors.sh >/tmp/validate-dogfood.log 2>&1 \
        || { cat /tmp/validate-dogfood.log >&2; fail "scripts/dogfood-detectors.sh: regressions detected"; }
    if command -v bats >/dev/null 2>&1 && [ -f tests/dogfood/test_driver.bats ]; then
        info "  running: tests/dogfood/test_driver.bats"
        bats tests/dogfood/test_driver.bats >/tmp/validate-dogfood-bats.log 2>&1 \
            || { cat /tmp/validate-dogfood-bats.log >&2; fail "tests/dogfood/test_driver.bats: failed"; }
    fi
    if command -v bats >/dev/null 2>&1 && [ -f tests/dogfood/test_adapter_layer.bats ]; then
        info "  running: tests/dogfood/test_adapter_layer.bats"
        bats tests/dogfood/test_adapter_layer.bats >/tmp/validate-dogfood-adapter.log 2>&1 \
            || { cat /tmp/validate-dogfood-adapter.log >&2; fail "tests/dogfood/test_adapter_layer.bats: failed"; }
    fi
}

# Verify-first discipline gate (issue #643): both helper scripts must exist,
# be executable, pass bash -n, and all 3 autospec-qa adapter files must
# reference the two new section headings + both script paths. The bats
# fixture suite is run as part of the gate.
check_qa_verify_first_discipline() {
    info "qa verify-first discipline: scripts + adapter lockstep"
    for s in scripts/qa-verify-finding.sh scripts/qa-cluster-coverage.sh \
             scripts/qa-finding-filter.sh; do
        [ -f "$s" ] || fail "$s: file missing (issue #643/#647)"
        [ -x "$s" ] || fail "$s: file not executable (issue #643/#647)"
        bash -n "$s" || fail "$s: bash syntax error (issue #643/#647)"
    done
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Verify-first discipline' "$trio" \
            || fail "$trio missing '## Verify-first discipline' section (issue #643)"
        grep -q '^## Cluster sizing' "$trio" \
            || fail "$trio missing '## Cluster sizing' section (issue #643)"
        grep -q 'qa-verify-finding\.sh' "$trio" \
            || fail "$trio missing reference to qa-verify-finding.sh (issue #643)"
        grep -q 'qa-cluster-coverage\.sh' "$trio" \
            || fail "$trio missing reference to qa-cluster-coverage.sh (issue #643)"
        grep -q 'qa-finding-filter\.sh' "$trio" \
            || fail "$trio missing reference to qa-finding-filter.sh (issue #647)"
        grep -q 'AUTOSPEC_QA_STRICT=1' "$trio" \
            || fail "$trio missing AUTOSPEC_QA_STRICT=1 rule (issue #647)"
    done
    if command -v bats >/dev/null 2>&1 && [ -f tests/qa/test_verify_first.bats ]; then
        info "  running: tests/qa/test_verify_first.bats"
        bats tests/qa/test_verify_first.bats >/tmp/validate-verify-first.log 2>&1 \
            || { cat /tmp/validate-verify-first.log >&2; fail "tests/qa/test_verify_first.bats: failed"; }
    fi
    if command -v bats >/dev/null 2>&1 && [ -f tests/qa/test_finding_filter.bats ]; then
        info "  running: tests/qa/test_finding_filter.bats"
        bats tests/qa/test_finding_filter.bats >/tmp/validate-finding-filter.log 2>&1 \
            || { cat /tmp/validate-finding-filter.log >&2; fail "tests/qa/test_finding_filter.bats: failed"; }
    fi
}

check_qa_incident_contract() {
    info "qa incident-regression contract: scripts + schema + adapter lockstep"
    local script="scripts/qa-incident-check.sh"
    local schema="schemas/autospec-production-incidents.schema.json"
    local bats="tests/qa/test_incident_check.bats"
    [ -f "$script" ] || fail "$script: file missing (issue #659)"
    [ -x "$script" ] || fail "$script: file not executable (issue #659)"
    bash -n "$script" || fail "$script: bash syntax error (issue #659)"
    [ -f "$schema" ] || fail "$schema: schema missing (issue #659)"
    [ -f "$bats" ] || fail "$bats: bats coverage missing (issue #659)"
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Production incident regression check' "$trio" \
            || fail "$trio missing '## Production incident regression check' section (issue #659)"
        grep -q 'qa-incident-check\.sh' "$trio" \
            || fail "$trio missing reference to qa-incident-check.sh (issue #659)"
        grep -q '\.autospec/production-incidents\.json' "$trio" \
            || fail "$trio missing reference to .autospec/production-incidents.json (issue #659)"
        grep -q 'never silently delete\|Never silently delete' "$trio" \
            || fail "$trio missing never-silently-delete rule (issue #659)"
    done
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats"
        bats "$bats" >/tmp/validate-incident-check.log 2>&1 \
            || { cat /tmp/validate-incident-check.log >&2; fail "$bats: failed"; }
    fi
}

check_qa_exhaustiveness_contract() {
    info "qa exhaustiveness flags: script + adapter lockstep (issue #660)"
    local script="scripts/qa-exhaustiveness-check.sh"
    local bats="tests/qa/test_exhaustiveness.bats"
    [ -f "$script" ] || fail "$script: file missing (issue #660)"
    [ -x "$script" ] || fail "$script: file not executable (issue #660)"
    bash -n "$script" || fail "$script: bash syntax error (issue #660)"
    [ -f "$bats" ] || fail "$bats: bats coverage missing (issue #660)"
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Exhaustiveness flags' "$trio" \
            || fail "$trio missing '## Exhaustiveness flags' section (issue #660)"
        grep -q 'qa-exhaustiveness-check\.sh' "$trio" \
            || fail "$trio missing reference to qa-exhaustiveness-check.sh (issue #660)"
        for flag in --every-spec-row --mutation-each --no-mock-each --exhaustive; do
            grep -q -- "$flag" "$trio" \
                || fail "$trio missing $flag documentation (issue #660)"
        done
    done
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats"
        bats "$bats" >/tmp/validate-exhaustiveness.log 2>&1 \
            || { cat /tmp/validate-exhaustiveness.log >&2; fail "$bats: failed"; }
    fi
}

check_qa_heal_loop_contract() {
    info "qa heal loop: scripts + adapter lockstep (issue #663)"
    local heal="scripts/qa-heal-loop.sh"
    local f2i="scripts/qa-finding-to-issue.sh"
    local bats="tests/qa/test_heal_loop.bats"
    [ -f "$heal" ] || fail "$heal: file missing (issue #663)"
    [ -x "$heal" ] || fail "$heal: file not executable (issue #663)"
    bash -n "$heal" || fail "$heal: bash syntax error (issue #663)"
    [ -f "$f2i" ] || fail "$f2i: file missing (issue #663)"
    [ -x "$f2i" ] || fail "$f2i: file not executable (issue #663)"
    bash -n "$f2i" || fail "$f2i: bash syntax error (issue #663)"
    [ -f "$bats" ] || fail "$bats: bats coverage missing (issue #663)"
    for trio in skills/autospec-qa/SKILL.md skills/autospec-qa/codex/prompt.md skills/autospec-qa/opencode/agent.md; do
        grep -q '^## Self-healing loop' "$trio" \
            || fail "$trio missing '## Self-healing loop' section (issue #663)"
        grep -q '^## --no-heal opt-out' "$trio" \
            || fail "$trio missing '## --no-heal opt-out' section (issue #663)"
        grep -q 'qa-heal-loop\.sh' "$trio" \
            || fail "$trio missing reference to qa-heal-loop.sh (issue #663)"
        grep -q 'qa-finding-to-issue\.sh' "$trio" \
            || fail "$trio missing reference to qa-finding-to-issue.sh (issue #663)"
        grep -q 'oscillation_detected' "$trio" \
            || fail "$trio missing 'oscillation_detected' terminator doc (issue #663)"
        grep -q 'AUTOSPEC_HEAL_MAX_ROUNDS' "$trio" \
            || fail "$trio missing AUTOSPEC_HEAL_MAX_ROUNDS doc (issue #663)"
        # Issue #711: lock in default-on + four opt-outs + shared driver.
        grep -q 'default-on' "$trio" \
            || fail "$trio missing 'default-on' lock-in (issue #711)"
        grep -q -- '--single-pass' "$trio" \
            || fail "$trio missing '--single-pass' opt-out alias (issue #711)"
        grep -q 'qa-no-heal\.flag' "$trio" \
            || fail "$trio missing 'qa-no-heal.flag' opt-out (issue #711)"
        grep -q 'lib/autospec-loop\.sh' "$trio" \
            || fail "$trio missing shared loop driver reference (issue #711)"
        grep -q 'qa-heal-summary\.md' "$trio" \
            || fail "$trio missing qa-heal-summary.md unified path (issue #711)"
        grep -q 'evidence_based_stop' "$trio" \
            || fail "$trio missing evidence_based_stop terminator (issue #711)"
    done
    # Issue #711: qa-heal-loop.sh must reference the shared driver and
    # support the new opt-out flag/path surface.
    grep -q 'scripts/lib/autospec-loop\.sh\|lib/autospec-loop\.sh' "$heal" \
        || fail "$heal missing reference to shared loop driver (issue #711)"
    grep -q -- '--single-pass' "$heal" \
        || fail "$heal missing --single-pass alias (issue #711)"
    grep -q 'qa-no-heal\.flag' "$heal" \
        || fail "$heal missing qa-no-heal.flag handling (issue #711)"
    grep -q 'qa-heal-summary\.md' "$heal" \
        || fail "$heal missing qa-heal-summary.md mirror (issue #711)"
    # New default-on bats (issue #711) — required.
    [ -f tests/qa/test_qa_heal_default.bats ] \
        || fail "tests/qa/test_qa_heal_default.bats: bats coverage missing (issue #711)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/qa/test_qa_heal_default.bats"
        bats tests/qa/test_qa_heal_default.bats >/tmp/validate-heal-default.log 2>&1 \
            || { cat /tmp/validate-heal-default.log >&2; fail "tests/qa/test_qa_heal_default.bats: failed"; }
    fi
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats"
        bats "$bats" >/tmp/validate-heal-loop.log 2>&1 \
            || { cat /tmp/validate-heal-loop.log >&2; fail "$bats: failed"; }
    fi
}

check_lint_heredoc_handling() {
    info "lint-implementation heredoc handling: tests/lint/test_complexity_heredoc.bats"
    [ -f tests/lint/test_complexity_heredoc.bats ] \
        || fail "tests/lint/test_complexity_heredoc.bats: bats coverage missing (issue #656)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/lint/test_complexity_heredoc.bats"
        bats tests/lint/test_complexity_heredoc.bats >/tmp/validate-lint-heredoc.log 2>&1 \
            || { cat /tmp/validate-lint-heredoc.log >&2; fail "tests/lint/test_complexity_heredoc.bats: failed"; }
    fi
}

# Quality-differential harness (issue #1023, spec §5 D4): the boilerplate guard
# (scripts/quality-differential.sh) plus its synthetic self-test negative pair
# and the >=3 refine-lens fixtures whose deterministic path is the documented
# KEEP-LLM failure.
check_quality_differential() {
    info "quality-differential harness: scripts/quality-differential.sh + tests/quality-differential.bats"
    [ -f scripts/quality-differential.sh ] \
        || fail "scripts/quality-differential.sh: missing (issue #1023)"
    [ -f tests/quality-differential.bats ] \
        || fail "tests/quality-differential.bats: bats coverage missing (issue #1023)"
    bash -n scripts/quality-differential.sh \
        || fail "scripts/quality-differential.sh: bash -n failed"
    local refine_count
    refine_count="$(ls tests/fixtures/quality-diff/refine-lenses/*/assert.sh 2>/dev/null | wc -l | tr -d ' ')"
    [ "${refine_count:-0}" -ge 3 ] \
        || fail "tests/fixtures/quality-diff/refine-lenses: need >=3 fixtures with assert.sh (got ${refine_count:-0})"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/quality-differential.bats"
        bats tests/quality-differential.bats >/tmp/validate-quality-diff.log 2>&1 \
            || { cat /tmp/validate-quality-diff.log >&2; fail "tests/quality-differential.bats: failed"; }
    fi
}

# Autonomous-mode contract (issue #662): the autospec + autospec-listen +
# autospec-define + autospec-run trios each carry an "## Autonomous mode"
# section, the autonomy-gate script is installed, and the bats coverage exists.
check_autospec_autonomous_contract() {
    info "autonomous contract: autospec + autospec-listen + autospec-define + autospec-run"
    for s in autospec autospec-listen autospec-define autospec-run; do
        for trio in SKILL.md codex/prompt.md opencode/agent.md; do
            f="skills/$s/$trio"
            [ -f "$f" ] || fail "$s: $trio missing"
            grep -q '^## Autonomous mode' "$f" \
                || fail "$s: $trio missing '## Autonomous mode' section"
        done
    done
    # autospec trio additionally carries explicit drafting + safety subsections.
    for trio in SKILL.md codex/prompt.md opencode/agent.md; do
        f="skills/autospec/$trio"
        grep -q '^### Autonomous spec drafting' "$f" \
            || fail "autospec: $trio missing '### Autonomous spec drafting' subsection"
        grep -q '^### Safety guardrails (autonomous)' "$f" \
            || fail "autospec: $trio missing '### Safety guardrails (autonomous)' subsection"
        grep -q 'autospec-autonomy-gate.sh' "$f" \
            || fail "autospec: $trio missing autospec-autonomy-gate.sh reference"
    done
    # autonomy-gate script invariants.
    [ -f scripts/autospec-autonomy-gate.sh ] \
        || fail "scripts/autospec-autonomy-gate.sh: file missing"
    [ -x scripts/autospec-autonomy-gate.sh ] \
        || fail "scripts/autospec-autonomy-gate.sh: not executable"
    bash -n scripts/autospec-autonomy-gate.sh \
        || fail "scripts/autospec-autonomy-gate.sh: bash syntax error"
    bash scripts/autospec-autonomy-gate.sh --help 2>/dev/null | grep -q '^Usage:' \
        || fail "scripts/autospec-autonomy-gate.sh --help did not print 'Usage:'"
    # Bats coverage.
    [ -f tests/autospec/test_autonomous.bats ] \
        || fail "tests/autospec/test_autonomous.bats: missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/autospec/test_autonomous.bats"
        bats tests/autospec/test_autonomous.bats >/tmp/validate-autonomous.log 2>&1 \
            || { cat /tmp/validate-autonomous.log >&2; fail "tests/autospec/test_autonomous.bats: failed"; }
    fi
}

check_autospec_refine_contract() {
    info "autospec-refine contract: trio + scripts + schema + bats (issue #672)"
    # Trio lockstep is enforced by the discover_skills loop in main(); here we
    # only assert the per-feature files referenced by the contract exist and
    # the bats coverage runs.
    local prompt_sh="scripts/refine-prompt.sh"
    local overview_sh="scripts/refine-render-overview.sh"
    local lens_llm_sh="scripts/refine-prompt-lens-llm.sh"
    local schema="schemas/autospec-refinement.schema.json"
    local loop_schema="schemas/autospec-refinement-loop.schema.json"
    [ -f "$prompt_sh" ] || fail "$prompt_sh: file missing (issue #672)"
    [ -x "$prompt_sh" ] || fail "$prompt_sh: not executable (issue #672)"
    bash -n "$prompt_sh" || fail "$prompt_sh: bash syntax error (issue #672)"
    # Shared matcher library (issue #707).
    matcher_lib="scripts/lib/extract-matchers.sh"
    [ -f "$matcher_lib" ] || fail "$matcher_lib: file missing (issue #707)"
    bash -n "$matcher_lib" || fail "$matcher_lib: bash syntax error (issue #707)"
    [ -f "$lens_llm_sh" ] || fail "$lens_llm_sh: file missing (issue #684)"
    [ -x "$lens_llm_sh" ] || fail "$lens_llm_sh: not executable (issue #684)"
    bash -n "$lens_llm_sh" || fail "$lens_llm_sh: bash syntax error (issue #684)"
    [ -f "$overview_sh" ] || fail "$overview_sh: file missing (issue #672)"
    [ -x "$overview_sh" ] || fail "$overview_sh: not executable (issue #672)"
    bash -n "$overview_sh" || fail "$overview_sh: bash syntax error (issue #672)"
    [ -f "$schema" ] || fail "$schema: file missing (issue #672)"
    [ -f "$loop_schema" ] || fail "$loop_schema: file missing (issue #682)"
    # Both schemas must parse as valid JSON Schema (issue #682).
    if command -v python3 >/dev/null 2>&1; then
        for s in "$schema" "$loop_schema"; do
            python3 - "$s" <<'PY' || fail "$s: schema parse/check_schema failed (issue #682)"
import json, sys
try:
    import jsonschema
except ImportError:
    sys.exit(0)
with open(sys.argv[1]) as f: sch = json.load(f)
jsonschema.Draft202012Validator.check_schema(sch)
PY
        done
    fi
    # autospec-refine trio existence (lockstep checked separately).
    for trio in skills/autospec-refine/SKILL.md skills/autospec-refine/codex/prompt.md skills/autospec-refine/opencode/agent.md; do
        [ -f "$trio" ] || fail "$trio: missing (issue #672)"
    done
    # Canonical ## Next steps section presence in autospec trio (issue #673).
    # Required for /autospec-refine --continue harvest contract.
    for trio in skills/autospec/SKILL.md skills/autospec/codex/prompt.md skills/autospec/opencode/agent.md; do
        grep -qE 'canonical `## Next steps` section' "$trio" \
            || fail "$trio: missing canonical '## Next steps' harvest-contract directive (issue #673)"
    done
    if command -v bats >/dev/null 2>&1; then
        if [ -d tests/refine ]; then
            for t in tests/refine/test_refine_*.bats; do
                [ -f "$t" ] || continue
                info "  running: $t"
                bats "$t" >/tmp/validate-refine.log 2>&1 \
                    || { cat /tmp/validate-refine.log >&2; fail "$t: failed"; }
            done
        fi
    fi
}

# Phase 6 run-summary contract (issue #683): the autospec trio must instruct
# Phase 6 to write `.autospec/run-summary.md` via the canonical helper, the
# helper script must exist + be executable + pass `bash -n`, and the bats
# coverage at tests/autospec/test_run_summary.bats must pass.
check_autospec_run_summary_contract() {
    info "autospec run-summary contract (Phase 6 → .autospec/run-summary.md)"
    for trio in SKILL.md codex/prompt.md opencode/agent.md; do
        f="skills/autospec/$trio"
        [ -f "$f" ] || fail "$f: required file missing"
        grep -q '\.autospec/run-summary\.md' "$f" \
            || fail "$f: Phase 6 missing .autospec/run-summary.md write directive"
        grep -q 'autospec-write-run-summary\.sh' "$f" \
            || fail "$f: Phase 6 missing autospec-write-run-summary.sh invocation"
        grep -q '(none — converged)' "$f" \
            || fail "$f: Phase 6 missing canonical convergence sentinel"
    done
    helper="scripts/autospec-write-run-summary.sh"
    [ -f "$helper" ] || fail "$helper: required helper missing"
    [ -x "$helper" ] || fail "$helper: not executable"
    bash -n "$helper" || fail "$helper: bash syntax error"
    bash "$helper" --help 2>/dev/null | grep -q '^Usage:' \
        || fail "$helper --help did not print a 'Usage:' line"
    bats_file="tests/autospec/test_run_summary.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-run-summary.log 2>&1 \
            || { cat /tmp/validate-run-summary.log >&2; fail "$bats_file: failed"; }
    fi
}

check_install_tests() {
    info "install tests: tests/install/*.sh"
    if [ -d tests/install ]; then
        for t in tests/install/*.sh; do
            [ -f "$t" ] || continue
            info "  running: $t"
            bash "$t" >/tmp/validate-install.log 2>&1 \
                || { cat /tmp/validate-install.log >&2; fail "$t: failed"; }
        done
    fi
}

# Parallel implementer worktree isolation contract (issue #690):
# every autospec-run trio file must carry a `### Parallel implementer worktree
# isolation` subsection; scripts/dispatch-implementer.sh must be present,
# executable, and bash -n clean; tests/autospec-run/test_parallel_dispatch.bats
# must exist and (if bats is available) pass.
check_autospec_parallel_dispatch_contract() {
    info "autospec parallel dispatch contract"
    for f in skills/autospec-run/SKILL.md skills/autospec-run/codex/prompt.md skills/autospec-run/opencode/agent.md; do
        grep -q '### Parallel implementer worktree isolation' "$f" \
            || fail "$f missing '### Parallel implementer worktree isolation' subsection"
        grep -q 'dispatch-implementer.sh' "$f" \
            || fail "$f missing reference to scripts/dispatch-implementer.sh"
    done
    helper="scripts/dispatch-implementer.sh"
    [ -f "$helper" ] || fail "$helper: required file missing"
    [ -x "$helper" ] || fail "$helper: file not executable"
    bash -n "$helper" || fail "$helper: bash syntax error"
    bats_file="tests/autospec-run/test_parallel_dispatch.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-parallel-dispatch.log 2>&1 \
            || { cat /tmp/validate-parallel-dispatch.log >&2; fail "$bats_file: failed"; }
    fi
}

# autospec-continue contract (issue #700): rate-limit + e2e bats coverage,
# skill trio presence, and helper script invariants. Lockstep is enforced by
# the duo-skill loop in main(); here we assert the per-feature scripts exist
# + are executable + bash -n clean, and run the new bats fixtures.
check_autospec_continue_contract() {
    info "autospec-continue contract: trio + scripts + bats (issue #700)"
    for trio in skills/autospec-continue/SKILL.md skills/autospec-continue/codex/prompt.md; do
        [ -f "$trio" ] || fail "$trio: missing (issue #700)"
    done
    for helper in scripts/autospec-continue.sh scripts/extract-conversational-recommendation.sh; do
        [ -f "$helper" ] || fail "$helper: file missing (issue #700)"
        [ -x "$helper" ] || fail "$helper: not executable (issue #700)"
        bash -n "$helper" || fail "$helper: bash syntax error (issue #700)"
    done
    # Shared matcher library (issue #707): both extract-conversational-
    # recommendation.sh and refine-prompt.sh::run_continue_loop() source this
    # so future matcher additions land in one place.
    matcher_lib="scripts/lib/extract-matchers.sh"
    [ -f "$matcher_lib" ] || fail "$matcher_lib: file missing (issue #707)"
    bash -n "$matcher_lib" || fail "$matcher_lib: bash syntax error (issue #707)"
    # Default-on continuous loop (issue #710): trio docs default-loop, opt-outs
    # via --no-loop and ~/.autospec/continue-no-loop.flag; orchestrator parses
    # both --no-loop and --once.
    for trio in skills/autospec-continue/SKILL.md skills/autospec-continue/codex/prompt.md skills/autospec-continue/opencode/agent.md; do
        [ -f "$trio" ] || continue
        grep -q '^## Continuous loop mode' "$trio" \
            || fail "$trio: missing '## Continuous loop mode' section (issue #710)"
        grep -q -- '--no-loop' "$trio" \
            || fail "$trio: missing --no-loop opt-out documentation (issue #710)"
        grep -q 'continue-no-loop\.flag' "$trio" \
            || fail "$trio: missing continue-no-loop.flag preference file (issue #710)"
    done
    grep -q -- '--no-loop|--once' scripts/autospec-continue.sh \
        || fail "scripts/autospec-continue.sh: --no-loop/--once flags missing (issue #710)"
    grep -q 'continue-no-loop\.flag' scripts/autospec-continue.sh \
        || fail "scripts/autospec-continue.sh: continue-no-loop.flag honor missing (issue #710)"
    grep -q 'autospec_loop_run' scripts/autospec-continue.sh \
        || fail "scripts/autospec-continue.sh: autospec_loop_run dispatch missing (issue #710)"
    if command -v bats >/dev/null 2>&1; then
        if [ -d tests/continue ]; then
            for t in tests/continue/test_continue_*.bats; do
                [ -f "$t" ] || continue
                info "  running: $t"
                bats "$t" >/tmp/validate-continue.log 2>&1 \
                    || { cat /tmp/validate-continue.log >&2; fail "$t: failed"; }
            done
        fi
    fi
}

# autospec --loop continuous-iteration contract (issue #708): the shared
# loop driver script must exist, be bash -n clean, and expose
# autospec_loop_run; the autospec trio (SKILL.md + codex + opencode) must
# carry a `## Continuous loop mode (--loop)` section in lockstep; the bats
# coverage at tests/autospec/test_autospec_loop.bats must exist and (if
# bats is available) pass.
check_autospec_loop_contract() {
    info "autospec --loop contract: shared driver + trio + bats (issue #708)"
    local lib="scripts/lib/autospec-loop.sh"
    [ -f "$lib" ] || fail "$lib: shared loop driver missing (issue #708)"
    bash -n "$lib" || fail "$lib: bash syntax error (issue #708)"
    grep -q '^autospec_loop_run()' "$lib" \
        || fail "$lib: autospec_loop_run() entry point missing (issue #708)"
    grep -q '^autospec_loop_harvest_next_prompt()' "$lib" \
        || fail "$lib: autospec_loop_harvest_next_prompt() missing (issue #708)"
    # refine-prompt.sh + autospec-continue.sh must source the shared lib so
    # there is a single source of truth for the loop driver.
    grep -q 'lib/autospec-loop\.sh' scripts/refine-prompt.sh \
        || fail "scripts/refine-prompt.sh: missing source of shared loop lib (issue #708)"
    grep -q 'lib/autospec-loop\.sh' scripts/autospec-continue.sh \
        || fail "scripts/autospec-continue.sh: missing source of shared loop lib (issue #708)"
    # Lockstep: each autospec trio file must carry the loop-mode section.
    for trio in skills/autospec/SKILL.md skills/autospec/codex/prompt.md skills/autospec/opencode/agent.md; do
        [ -f "$trio" ] || fail "$trio: missing (issue #708)"
        grep -q '^## Continuous loop mode (--loop)' "$trio" \
            || fail "$trio: missing '## Continuous loop mode (--loop)' section (issue #708)"
        grep -q 'autospec_loop_run\|lib/autospec-loop\.sh' "$trio" \
            || fail "$trio: loop section must reference the shared driver (issue #708)"
        grep -q 'evidence_based_stop' "$trio" \
            || fail "$trio: loop section must enumerate termination conditions (issue #708)"
    done
    local bats_file="tests/autospec/test_autospec_loop.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing (issue #708)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-autospec-loop.log 2>&1 \
            || { cat /tmp/validate-autospec-loop.log >&2; fail "$bats_file: failed"; }
    fi
}

# Atomic cross-machine issue claim CAS guard (issue #871, spec
# docs/specs/2026-06-02-atomic-cross-machine-issue-claim-design.md §Testing).
#
# Prevents regression of the whole CAS-claim effort (#872/#873/#874): the hot
# monitor loop must route every claim through claim-issue.sh, never through an
# inline blind check-then-act label swap. This guard fails if any of the three
# autospec-run harness trio files (SKILL.md, opencode/agent.md, codex/prompt.md)
# still contains an unguarded `gh issue edit ... --add-label in-progress-by-bot`
# in the hot loop — i.e. an add-label swap on a line that does NOT also route
# through claim-issue.sh. It also asserts each trio file references
# claim-issue.sh in its claim section so the documented CAS call site cannot be
# silently dropped.
#
# Lockstep byte-identity of the rewritten claim prose is covered separately by
# check_lockstep (run per-skill in main()). The two mocked-`gh` PATH-shadow
# simulations from #873/#874 (contended-claim ids 100/101 + stale-lease reclaim)
# are executed here when bats is available so the CAS correctness argument is
# part of the validate flow, mirroring check_supersession_contract.
check_claim_cas_guard() {
    info "claim CAS guard: autospec-run trio (issue #871)"
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        f="skills/autospec-run/$trio"
        [ -f "$f" ] || fail "$f: required file missing"
        # (1) The claim section must reference claim-issue.sh (the SOLE CAS path).
        grep -q 'claim-issue\.sh' "$f" \
            || fail "$f: claim section missing reference to claim-issue.sh (issue #871)"
        # (2) No unguarded check-then-act swap: any line that performs
        #     `gh issue edit ... --add-label in-progress-by-bot` must also route
        #     through claim-issue.sh. A bare add-label swap is the regression.
        unguarded="$(grep -nE 'gh issue edit .*--add-label in-progress-by-bot' "$f" \
            | grep -v 'claim-issue\.sh' || true)"
        if [ -n "$unguarded" ]; then
            printf '%s\n' "$unguarded" >&2
            fail "$f: unguarded check-then-act 'gh issue edit --add-label in-progress-by-bot' outside the claim-issue.sh CAS path (issue #871)"
        fi
    done
    # (3) The two mocked-`gh` PATH-shadow simulations (contended-claim ids
    #     100/101 + stale-lease reclaim) encode the CAS correctness argument.
    [ -f tests/fixtures/gh-mock/gh ] \
        || fail "tests/fixtures/gh-mock/gh: PATH-shadow mocked-gh fixture missing (issue #871)"
    for bats_file in \
        tests/unit/test_autospec_claim_id_tiebreak.bats \
        tests/unit/test_autospec_stale_lease_reclaim.bats
    do
        [ -f "$bats_file" ] || fail "$bats_file: mocked-gh simulation missing (issue #871)"
        if command -v bats >/dev/null 2>&1; then
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-claim-cas-guard.log 2>&1 \
                || { cat /tmp/validate-claim-cas-guard.log >&2; fail "$bats_file: failed (issue #871)"; }
        fi
    done
}

# Watchdog orphaned-worktree GC (crash-resume design, Child 3 / issue #883).
# Enforces the data-integrity contract structurally:
#   - gc_orphaned_worktrees is defined and invoked once per cycle.
#   - Pruning uses `git worktree remove --force`, never `rm -rf` of a worktree.
#   - The script passes `bash -n`.
#   - A bats fixture under tests/resume/ covers the three GC outcomes.
check_watchdog_worktree_gc() {
    info "watchdog orphaned-worktree GC (issue #883)"
    local wd="scripts/autospec-watchdog.sh"
    [ -f "$wd" ] || fail "$wd: missing"
    bash -n "$wd" || fail "$wd: bash syntax error (issue #883)"
    grep -q 'gc_orphaned_worktrees()' "$wd" \
        || fail "$wd must define gc_orphaned_worktrees() (issue #883)"
    grep -q '^gc_orphaned_worktrees$\|^    gc_orphaned_worktrees$' "$wd" \
        || fail "$wd must invoke gc_orphaned_worktrees once per cycle (issue #883)"
    grep -q 'git worktree remove --force' "$wd" \
        || fail "$wd GC must prune via 'git worktree remove --force' (issue #883)"
    # Data-integrity guard: the GC must never rm -rf a worktree path. Allow no
    # `rm -rf` anywhere in the watchdog (it has never needed one).
    if grep -vE '^[[:space:]]*#' "$wd" | grep -qE 'rm[[:space:]]+-rf'; then
        fail "$wd must never 'rm -rf' a worktree path; use 'git worktree remove --force' (issue #883)"
    fi
    grep -q 'log --not --remotes' "$wd" \
        || fail "$wd GC must gate on 'git -C <wt> log --not --remotes' for un-pushed commits (issue #883)"
    [ -f tests/resume/test_watchdog_gc.bats ] \
        || fail "tests/resume/test_watchdog_gc.bats: GC bats fixture missing (issue #883)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: tests/resume/test_watchdog_gc.bats"
        bats tests/resume/test_watchdog_gc.bats >/tmp/validate-watchdog-gc.log 2>&1 \
            || { cat /tmp/validate-watchdog-gc.log >&2; fail "tests/resume/test_watchdog_gc.bats: failed (issue #883)"; }
    fi
}

# AGENTS.md git-hygiene section (issue #962, spec §D5): AGENTS.md must carry
# the `## Git hygiene (agents)` section with the five sub-rules (fetch-before-
# branch; primary checkout read-only; fresh-or-verified-clean worktrees;
# cleanup after merge + prune; PR-aware ladder as standard behavior) and a
# pointer to worktree-guard.sh.
check_agents_md_git_hygiene() {
    info "AGENTS.md git hygiene section (issue #962 §D5)"
    [ -f AGENTS.md ] || fail "AGENTS.md missing at repo root"
    grep -q '^## Git hygiene (agents)$' AGENTS.md \
        || fail "AGENTS.md missing '## Git hygiene (agents)' section (issue #962 §D5)"
    grep -q 'Fetch-before-branch\|fetch-before-branch' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing fetch-before-branch rule (§D5)"
    grep -q 'primary checkout' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing primary-checkout read-only rule (§D5)"
    grep -q 'fresh-or-verified-clean\|Fresh-or-verified-clean' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing fresh-or-verified-clean worktrees rule (§D5)"
    grep -q 'git worktree remove' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing 'git worktree remove' cleanup rule (§D5)"
    grep -q 'git worktree prune' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing 'git worktree prune' cleanup rule (§D5)"
    grep -q 'open-pr\|branch-only\|fresh' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing PR-aware ladder states (§D5)"
    grep -q 'worktree-guard\.sh' AGENTS.md \
        || fail "AGENTS.md git hygiene section missing pointer to worktree-guard.sh (§D5)"
}

# Define spec-PR worktree routing (issue #962, spec §D4): the autospec-define
# trio must route the spec-PR commit through worktree-guard.sh create to a temp
# worktree (/tmp/wt-spec-<slug>), never the primary checkout. The define-spec-pr
# sentinel block must be present in all three trio files in lockstep.
check_define_spec_worktree_routing() {
    info "define spec-PR worktree routing (issue #962 §D4)"
    for trio in \
        skills/autospec-define/SKILL.md \
        skills/autospec-define/codex/prompt.md \
        skills/autospec-define/opencode/agent.md
    do
        [ -f "$trio" ] || fail "$trio: required file missing (issue #962 §D4)"
        grep -q 'worktree-guard.sh' "$trio" \
            || fail "$trio: missing worktree-guard.sh reference (issue #962 §D4)"
        grep -q '/tmp/wt-spec-' "$trio" \
            || fail "$trio: missing /tmp/wt-spec-<slug> temp worktree path (issue #962 §D4)"
        grep -q 'MUST exit 0 before any edit' "$trio" \
            || fail "$trio: missing 'MUST exit 0 before any edit' assert directive (issue #962 §D4)"
        grep -q 'resolve-branch' "$trio" \
            || fail "$trio: missing resolve-branch call (issue #962 §D4)"
        grep -q 'define-spec-pr:begin' "$trio" \
            || fail "$trio: missing define-spec-pr:begin sentinel (issue #962 §D4)"
        grep -q 'define-spec-pr:end' "$trio" \
            || fail "$trio: missing define-spec-pr:end sentinel (issue #962 §D4)"
        grep -q 'git worktree remove' "$trio" \
            || fail "$trio: missing 'git worktree remove' cleanup rule (issue #962 §D4)"
        grep -q 'git worktree prune' "$trio" \
            || fail "$trio: missing 'git worktree prune' cleanup rule (issue #962 §D4)"
    done
    # bats coverage file must exist.
    local bats_file="tests/phase4/test_define_spec_worktree.sh"
    [ -f "$bats_file" ] \
        || fail "$bats_file: bats coverage missing (issue #962 §D4)"
    info "  running: $bats_file"
    bash "$bats_file" >/tmp/validate-define-spec-worktree.log 2>&1 \
        || { cat /tmp/validate-define-spec-worktree.log >&2; fail "$bats_file: failed (issue #962)"; }
}

# D1 token-baseline staleness check (issue #1017).
# Warn-only: never fails the validate run; a stale baseline just prints a
# reminder to regenerate with `bash scripts/skill-token-report.sh`.
check_token_baseline_fresh() {
    info "token baseline staleness check (issue #1017, warn-only)"
    local baseline="docs/reports/skill-token-baseline.md"
    local script="scripts/skill-token-report.sh"

    if [ ! -f "$baseline" ]; then
        printf 'validate: WARN — %s not found; run: bash %s --update-baseline\n' \
            "$baseline" "$script" >&2
        return 0
    fi

    if [ ! -f "$script" ]; then
        printf 'validate: WARN — %s not found; cannot check baseline staleness\n' \
            "$script" >&2
        return 0
    fi

    local fresh
    fresh="$(bash "$script" 2>/dev/null)" || {
        printf 'validate: WARN — %s failed; skipping staleness check\n' "$script" >&2
        return 0
    }

    # Extract only the table lines from the baseline (between baseline markers)
    local committed
    committed="$(awk '/<!-- baseline:begin -->/{f=1;next} /<!-- baseline:end -->/{f=0} f' "$baseline")"

    if [ "$fresh" != "$committed" ]; then
        printf 'validate: WARN — %s is stale; run: bash %s --update-baseline\n' \
            "$baseline" "$script" >&2
    fi
}

# Golden-snapshot gate (D2, issue #1019): for every skill with a stored golden
# in tests/fixtures/skill-goldens/<skill>.SKILL.md.sha256, expand the current
# SKILL.md through scripts/expand-skill-blocks.sh and compare sha256 to the
# golden. A mismatch means the bytes changed without an approved golden update.
# Missing golden for an existing SKILL.md -> FAIL (fail closed).
# Missing expander script -> FAIL (fail closed).
# No RETURN traps; if/then/fi for one-sided conditionals (repo bash rules).
# Helper: gate one trio member (src file -> golden) through the expander.
# Appends the member label to the caller's $failed accumulator (by echoing it).
# Echoes nothing on success. On mismatch, prints a FAIL diagnostic to stderr and
# echoes the failing label to stdout.
# Args: <expander> <golden_dir> <skill> <src_file> <golden_suffix>
# golden_suffix is one of: SKILL.md | codex.prompt.md | opencode.agent.md
# SKILL.md goldens are required (fail closed). codex/opencode goldens are
# required only when the member source carries a marker (TOKR1-003); a markered
# member with no golden fails closed.
gate_block_member() {
    _gbm_expander="$1"
    _gbm_golden_dir="$2"
    _gbm_skill="$3"
    _gbm_src="$4"
    _gbm_suffix="$5"
    [ -f "$_gbm_src" ] || return 0
    _gbm_golden="$_gbm_golden_dir/${_gbm_skill}.${_gbm_suffix}.sha256"
    if [ ! -f "$_gbm_golden" ]; then
        if [ "$_gbm_suffix" = "SKILL.md" ]; then
            fail "check_block_expansion: no golden for $_gbm_skill (tests/fixtures/skill-goldens/${_gbm_skill}.${_gbm_suffix}.sha256 missing — fail closed)"
        fi
        # codex/opencode member: a golden is required iff the source is markered.
        if grep -q '<!-- autospec-block:' "$_gbm_src" 2>/dev/null; then
            fail "check_block_expansion: markered member $_gbm_src has no golden (tests/fixtures/skill-goldens/${_gbm_skill}.${_gbm_suffix}.sha256 missing — fail closed)"
        fi
        return 0
    fi
    _gbm_expected="$(cat "$_gbm_golden" | tr -d '[:space:]')"
    _gbm_got="$(bash "$_gbm_expander" "$_gbm_src" | shasum -a 256 | cut -d' ' -f1)"
    if [ "$_gbm_got" != "$_gbm_expected" ]; then
        printf 'validate: FAIL — check_block_expansion: %s %s expanded sha256 mismatch (got %s, want %s)\n' \
            "$_gbm_skill" "$_gbm_suffix" "$_gbm_got" "$_gbm_expected" >&2
        printf '%s.%s' "$_gbm_skill" "$_gbm_suffix"
    fi
}

check_block_expansion() {
    info "block-expansion golden gate: tests/fixtures/skill-goldens/"
    local expander="$REPO_ROOT/scripts/expand-skill-blocks.sh"
    if [ ! -f "$expander" ]; then
        fail "check_block_expansion: scripts/expand-skill-blocks.sh missing (fail closed)"
    fi
    local golden_dir="$REPO_ROOT/tests/fixtures/skill-goldens"
    local failed=""
    local m
    for skill_dir in skills/*/; do
        [ -d "$skill_dir" ] || continue
        local skill
        skill="$(basename "$skill_dir")"
        # SKILL.md is required for every skill (fail closed on missing golden).
        if [ -f "$skill_dir/SKILL.md" ]; then
            m="$(gate_block_member "$expander" "$golden_dir" "$skill" "$skill_dir/SKILL.md" "SKILL.md")"
            failed="${failed}${m:+ $m}"
        fi
        # codex/opencode trio members: gated whenever a golden exists, and
        # required (fail closed) whenever the member source carries a marker.
        m="$(gate_block_member "$expander" "$golden_dir" "$skill" "$skill_dir/codex/prompt.md" "codex.prompt.md")"
        failed="${failed}${m:+ $m}"
        m="$(gate_block_member "$expander" "$golden_dir" "$skill" "$skill_dir/opencode/agent.md" "opencode.agent.md")"
        failed="${failed}${m:+ $m}"
    done
    if [ -n "$failed" ]; then
        fail "check_block_expansion: sha256 mismatch for members:$failed"
    fi
    info "block-expansion golden gate: all skills + members OK"
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
    check_enforcement_defaults_section
    check_codex_skills_install
    check_shared_script_install

    check_agents_md_subagent_section
    check_agents_md_subagent_matrix
    check_autospec_listen_files
    check_examples_dir
    check_governance_headings
    check_autospec_stl_design_guardrails
    check_existing_spec_mode
    check_lint_issue_helpers
    check_lint_implementation_helpers
    check_implementer_contract
    check_lint_heredoc_handling
    check_quality_differential
    check_usage_limit_helper
    check_supersession_contract
    check_ship_completeness
    check_phase4_guardian_block_lockstep
    check_phase1_bounded_context_contract
    check_phase4_issue_start_summary
    check_phase4_immediate_next_issue_pickup
    check_autospec_run_continuation_contract
    check_phase4_single_agent_discipline
    check_phase4_adaptive_retry
    check_phase4_full_test_suite_gate
    check_phase4_cost_epic_parity_lockstep
    check_docs_drift_gate_regen_conditional_parity
    check_worktree_ladder_assert_parity
    check_autospec_sweep_config_contract
    check_autospec_fleet_scripts
    check_fleet_gui_subcommand_lockstep
    check_team_personality_contract
    check_autospec_run_priority_sort_lockstep
    check_autospec_run_regression_review_lockstep
    check_claim_cas_guard
    check_autospec_review_skill_present
    check_autospec_review_tier_a_directives
    check_autospec_test_skill_present
    check_autospec_playwright_skill_present
    check_autospec_qa_contract
    check_brute_force_rule_ids
    check_dogfood_detectors
    check_qa_verify_first_discipline
    check_qa_incident_contract
    check_qa_exhaustiveness_contract
    check_qa_heal_loop_contract
    check_autospec_release_contract
    check_qa_verdict_contract
    check_release_verdict_script
    check_docs_amendment_presence
    check_autospec_autonomous_contract
    check_autospec_refine_contract
    check_autospec_continue_contract
    check_autospec_loop_contract
    check_autospec_run_summary_contract
    check_install_tests
    check_phase4_tests
    check_autospec_parallel_dispatch_contract
    check_autospec_explore_implementer_base
    check_loop_handoff_harness_awareness
    check_autospec_explore_researchers_deterministic
    check_autospec_explore_researchers_llm
    check_autospec_explore_specialists_discovery
    check_autospec_explore_contract
    check_explore_trio_worktree_assert
    check_autospec_explore_discovery_contract
    check_autospec_explore_spec_first_contract
    check_autospec_explore_qa_gate_contract
    check_autospec_release_area_contract
    check_release_trio_worktree_assert
    check_autospec_sweep_area_contract
    check_autospec_qa_cluster_contract
    check_autospec_qa_bug_class_contract
    check_autospec_resume_contract
    check_palette_single_source
    check_autospec_doc_contract
    check_watchdog_worktree_gc
    check_qa_documentation_gate
    check_agents_md_git_hygiene
    check_define_spec_worktree_routing
    check_token_baseline_fresh
    check_block_expansion
    check_claim_guard_contract

    # Top-level installer / uninstaller (introduced in PR #11) — only check syntax
    # if present; absence is OK before that PR lands.
    check_bash_syntax "install.sh"
    check_bash_syntax "uninstall.sh"

    info "OK — all validation checks passed."
}

# Issue #730: autospec-qa per-area cluster dispatch contract.
# Enforces:
#   - 8 canonical cluster files under skills/autospec-qa/clusters/.
#   - scripts/qa-cluster-dispatch.sh exists, executable, bash -n clean.
#   - all 3 autospec-qa adapter files reference the cluster dispatcher.
#   - tests/qa/test_qa_cluster_dispatch.bats exists.
check_autospec_qa_cluster_contract() {
    info "autospec-qa per-area cluster dispatch contract (issue #730)"
    local clusters_dir="skills/autospec-qa/clusters"
    local dispatcher="scripts/qa-cluster-dispatch.sh"
    local bats_file="tests/qa/test_qa_cluster_dispatch.bats"

    [ -d "$clusters_dir" ] || fail "$clusters_dir: missing cluster directory (issue #730)"
    for c in spec-traceability functional-coverage backend-integration \
             reliability-contract legacy-and-cleanup \
             benchmark-and-outsourcing accessibility-and-responsive \
             production-incidents; do
        [ -f "$clusters_dir/$c.md" ] \
            || fail "$clusters_dir/$c.md: missing cluster file (issue #730)"
    done

    [ -f "$dispatcher" ] || fail "$dispatcher: missing (issue #730)"
    [ -x "$dispatcher" ] || fail "$dispatcher: not executable (issue #730)"
    bash -n "$dispatcher" || fail "$dispatcher: bash -n failed (issue #730)"

    for trio in skills/autospec-qa/SKILL.md \
                skills/autospec-qa/codex/prompt.md \
                skills/autospec-qa/opencode/agent.md; do
        grep -q 'qa-cluster-dispatch\.sh' "$trio" \
            || fail "$trio: missing reference to qa-cluster-dispatch.sh (issue #730)"
        grep -q 'skills/autospec-qa/clusters/' "$trio" \
            || fail "$trio: missing reference to clusters/ directory (issue #730)"
    done

    [ -f "$bats_file" ] || fail "$bats_file: missing (issue #730)"
}

# Issue #737: autospec-qa bug-class sibling sweep contract.
# Enforces:
#   - scripts/qa-bug-class-sweep.sh exists, executable, bash -n clean.
#   - all 3 autospec-qa trio files reference qa-bug-class-sweep.sh and
#     the per-class issue-filing semantics.
#   - heal-loop references parent_fingerprint grouping.
#   - tests/qa/test_qa_bug_class_sweep.bats exists.
check_autospec_qa_bug_class_contract() {
    info "autospec-qa bug-class sibling sweep contract (issue #737)"
    local helper="scripts/qa-bug-class-sweep.sh"
    local bats_file="tests/qa/test_qa_bug_class_sweep.bats"
    local heal="scripts/qa-heal-loop.sh"

    [ -f "$helper" ] || fail "$helper: missing (issue #737)"
    [ -x "$helper" ] || fail "$helper: not executable (issue #737)"
    bash -n "$helper" || fail "$helper: bash -n failed (issue #737)"

    for trio in skills/autospec-qa/SKILL.md \
                skills/autospec-qa/codex/prompt.md \
                skills/autospec-qa/opencode/agent.md; do
        grep -q 'qa-bug-class-sweep\.sh' "$trio" \
            || fail "$trio: missing reference to qa-bug-class-sweep.sh (issue #737)"
        grep -q 'parent_fingerprint' "$trio" \
            || fail "$trio: missing parent_fingerprint contract (issue #737)"
        grep -q 'AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE' "$trio" \
            || fail "$trio: missing confidence-threshold env var (issue #737)"
    done

    grep -q 'parent_fingerprint' "$heal" \
        || fail "$heal: must group by parent_fingerprint (issue #737)"

    [ -f "$bats_file" ] || fail "$bats_file: missing (issue #737)"
}

# Issue #723: harness-aware loop dispatcher contract. Every loop-using
# script must source scripts/lib/autospec-harness-detect.sh so the
# `/autospec --autonomous` handoff form matches the active AI harness
# (Claude Code / Codex CLI / OpenCode).
check_loop_handoff_harness_awareness() {
    info "loop handoff harness-awareness (issue #723)"
    local helper="scripts/lib/autospec-harness-detect.sh"
    [ -f "$helper" ] || fail "$helper: missing — required for harness-aware loop handoff"
    bash -n "$helper" || fail "$helper: bash syntax error"
    # Each loop-using consumer must source the shared helper rather than
    # carrying its own claude/autospec resolution.
    local consumers=(
        "scripts/refine-prompt.sh"
        "scripts/autospec-continue.sh"
        "scripts/qa-heal-loop.sh"
        "scripts/refine-prompt-lens-llm.sh"
    )
    for c in "${consumers[@]}"; do
        [ -f "$c" ] || fail "$c: missing"
        grep -q "lib/autospec-harness-detect.sh" "$c" \
            || fail "$c: must source scripts/lib/autospec-harness-detect.sh (issue #723)"
    done
    # Bats coverage must exist.
    [ -f "tests/lib/test_autospec_harness_detect.bats" ] \
        || fail "tests/lib/test_autospec_harness_detect.bats: missing (issue #723)"
    [ -f "tests/refine/test_refine_loop_codex.bats" ] \
        || fail "tests/refine/test_refine_loop_codex.bats: missing (issue #723)"
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
    if [ -f tests/e2e/test_autospec_fleet_dry_run.bats ] && [ -d tests/fixtures/fleet ]; then
        info "e2e tests: tests/e2e/test_autospec_fleet_dry_run.bats"
        AUTOSPEC_FLEET_LIVE_E2E=0 bats tests/e2e/test_autospec_fleet_dry_run.bats >/tmp/validate-fleet-e2e.log 2>&1 \
            || { cat /tmp/validate-fleet-e2e.log >&2; fail "tests/e2e/test_autospec_fleet_dry_run.bats: failed"; }
    fi
}

# autospec-explore implementer PR-base integration (issue #718):
# the Phase 4 implementer prompt must reference .autospec/explore-mode.json,
# encode sandbox-base resolution, and a main-merge refusal exit identifier.
# The autospec-run trio must reference both the explore-mode file and the
# phase4-implementer.md prompt in lockstep. Runs tests/explore bats fixtures
# when bats is on PATH.
check_autospec_explore_implementer_base() {
    info "autospec-explore implementer PR-base integration"
    impl="skills/autospec-run/prompts/phase4-implementer.md"
    [ -f "$impl" ] || fail "$impl: required file missing"
    grep -F '.autospec/explore-mode.json' "$impl" >/dev/null \
        || fail "$impl missing .autospec/explore-mode.json reference"
    grep -F 'EXPLORE_BASE' "$impl" >/dev/null \
        || fail "$impl missing EXPLORE_BASE sandbox-base resolution"
    grep -F 'gh pr create --base "$EXPLORE_BASE"' "$impl" >/dev/null \
        || fail "$impl missing 'gh pr create --base \"\$EXPLORE_BASE\"' snippet"
    grep -F 'code_health:explore_main_merge_refused' "$impl" >/dev/null \
        || fail "$impl missing code_health:explore_main_merge_refused refusal identifier"
    for trio in \
        skills/autospec-run/SKILL.md \
        skills/autospec-run/codex/prompt.md \
        skills/autospec-run/opencode/agent.md
    do
        grep -F '.autospec/explore-mode.json' "$trio" >/dev/null \
            || fail "$trio missing .autospec/explore-mode.json reference (lockstep)"
        grep -F 'phase4-implementer.md' "$trio" >/dev/null \
            || fail "$trio missing reference to phase4-implementer.md (lockstep)"
        grep -F 'code_health:explore_main_merge_refused' "$trio" >/dev/null \
            || fail "$trio missing code_health:explore_main_merge_refused (lockstep)"
    done
    bats_file="tests/explore/test_explore_implementer_base.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-explore-impl-base.log 2>&1 \
            || { cat /tmp/validate-explore-impl-base.log >&2; fail "$bats_file: failed"; }
    fi
}

# autospec-explore research cycle + 4 deterministic researchers (issue #719):
# all 4 researcher scripts + the aggregator must exist, be executable, and
# pass `bash -n`. Runs tests/explore/test_explore_researchers.bats and
# tests/explore/test_explore_research_cycle.bats when bats is on PATH.
check_autospec_explore_researchers_deterministic() {
    info "autospec-explore deterministic researchers + cycle aggregator"
    local aggregator="scripts/explore-research-cycle.sh"
    [ -f "$aggregator" ] || fail "$aggregator: required file missing"
    [ -x "$aggregator" ] || fail "$aggregator: file not executable"
    bash -n "$aggregator" || fail "$aggregator: bash syntax error"
    for r in spec-vs-code prior-reports codebase-signals open-issues; do
        local script="scripts/explore-research/$r.sh"
        [ -f "$script" ] || fail "$script: required file missing"
        [ -x "$script" ] || fail "$script: file not executable"
        bash -n "$script" || fail "$script: bash syntax error"
    done
    for bats_file in \
        tests/explore/test_explore_researchers.bats \
        tests/explore/test_explore_research_cycle.bats
    do
        [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
        if command -v bats >/dev/null 2>&1; then
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-explore-researchers.log 2>&1 \
                || { cat /tmp/validate-explore-researchers.log >&2; fail "$bats_file: failed"; }
        fi
    done
}

# autospec-explore LLM-heavy researchers + internet safety (issue #720):
# both LLM researcher scripts (source-analysis.sh, internet.sh) plus the
# explore-internet-safety.sh helper must exist, be executable, and pass
# `bash -n`. Runs tests/explore/test_explore_researchers_llm.bats and
# tests/explore/test_explore_internet_safety.bats when bats is on PATH.
check_autospec_explore_researchers_llm() {
    info "autospec-explore LLM-heavy researchers + internet safety (issue #720)"
    local safety_lib="scripts/lib/explore-internet-safety.sh"
    [ -f "$safety_lib" ] || fail "$safety_lib: required helper missing"
    bash -n "$safety_lib" || fail "$safety_lib: bash syntax error"
    for r in source-analysis internet; do
        local script="scripts/explore-research/$r.sh"
        [ -f "$script" ] || fail "$script: required file missing"
        [ -x "$script" ] || fail "$script: file not executable"
        bash -n "$script" || fail "$script: bash syntax error"
    done
    for bats_file in \
        tests/explore/test_explore_researchers_llm.bats \
        tests/explore/test_explore_internet_safety.bats
    do
        [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
        if command -v bats >/dev/null 2>&1; then
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-explore-llm.log 2>&1 \
                || { cat /tmp/validate-explore-llm.log >&2; fail "$bats_file: failed"; }
        fi
    done
}

# autospec-explore domain-specialist roster discovery (Issue E1 #1082):
# the deterministic signal-scan script + the new specialists schema must
# exist, be executable / well-formed, and the bats coverage must pass when
# bats is on PATH. The schema must compile with ajv when available.
check_autospec_explore_specialists_discovery() {
    info "autospec-explore domain-specialist roster discovery (Issue E1)"
    local scan="scripts/explore-specialist-scan.sh"
    [ -f "$scan" ] || fail "$scan: required file missing"
    [ -x "$scan" ] || fail "$scan: file not executable"
    bash -n "$scan" || fail "$scan: bash syntax error"
    local schema="schemas/autospec-explore-specialists.schema.json"
    [ -f "$schema" ] || fail "$schema: specialists roster schema missing"
    if command -v jq >/dev/null 2>&1; then
        jq -e '.properties.domains and .properties.suggested_specialists' "$schema" >/dev/null \
            || fail "$schema: missing domains/suggested_specialists contract"
    fi
    if command -v ajv >/dev/null 2>&1; then
        ajv compile -s "$schema" --spec=draft2020 >/dev/null 2>&1 \
            || fail "$schema: does not compile with ajv"
    fi
    local bats_file="tests/explore/test_explore_specialists.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-explore-specialists.log 2>&1 \
            || { cat /tmp/validate-explore-specialists.log >&2; fail "$bats_file: failed"; }
    fi
}

# autospec-explore orchestrator contract (issue #721): top-level
# scripts/autospec-explore.sh exists, is executable, bash -n clean, sources
# the shared loop driver, references all explore subcomponents, and the
# e2e bats fixture passes when bats is available. Also enforces the
# autospec-explore trio lockstep already covered by the multi-harness
# scanner plus references to the shared loop driver in adapters.
check_autospec_explore_contract() {
    info "autospec-explore orchestrator + loop integration + e2e (issue #721)"
    local orch="scripts/autospec-explore.sh"
    [ -f "$orch" ] || fail "$orch: top-level orchestrator missing"
    [ -x "$orch" ] || fail "$orch: file not executable"
    bash -n "$orch" || fail "$orch: bash syntax error"
    grep -q 'lib/autospec-loop\.sh' "$orch" \
        || fail "$orch: must source scripts/lib/autospec-loop.sh"
    grep -q 'lib/autospec-harness-detect\.sh' "$orch" \
        || fail "$orch: must source scripts/lib/autospec-harness-detect.sh"
    grep -q 'explore-sandbox\.sh' "$orch" \
        || fail "$orch: must invoke scripts/explore-sandbox.sh"
    grep -q 'explore-research-cycle\.sh' "$orch" \
        || fail "$orch: must invoke scripts/explore-research-cycle.sh"
    grep -q 'explore-stop\.flag' "$orch" \
        || fail "$orch: must check ~/.autospec/explore-stop.flag operator escape"
    grep -q 'explore-summary\.md' "$orch" \
        || fail "$orch: must write .autospec/explore-summary.md"
    grep -q 'explore-loop\.json' "$orch" \
        || fail "$orch: must write .autospec/explore-loop.json"

    # All 6 researchers + 3 explore scripts must exist + bash -n clean.
    for r in spec-vs-code prior-reports codebase-signals open-issues source-analysis internet; do
        local script="scripts/explore-research/$r.sh"
        [ -f "$script" ] || fail "$script: required researcher missing"
        [ -x "$script" ] || fail "$script: file not executable"
        bash -n "$script" || fail "$script: bash syntax error"
    done
    for s in scripts/explore-sandbox.sh scripts/explore-research-cycle.sh scripts/autospec-explore.sh; do
        [ -f "$s" ] || fail "$s: required script missing"
        [ -x "$s" ] || fail "$s: file not executable"
        bash -n "$s" || fail "$s: bash syntax error"
    done

    # Adapter trio must reference the shared driver.
    for trio in \
        skills/autospec-explore/SKILL.md \
        skills/autospec-explore/codex/prompt.md \
        skills/autospec-explore/opencode/agent.md
    do
        [ -f "$trio" ] || fail "$trio: required adapter file missing"
        grep -q 'lib/autospec-loop\.sh' "$trio" \
            || fail "$trio: adapter must reference scripts/lib/autospec-loop.sh"
    done

    local bats_file="tests/explore/test_explore_e2e.bats"
    [ -f "$bats_file" ] || fail "$bats_file: e2e bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-explore-e2e.log 2>&1 \
            || { cat /tmp/validate-explore-e2e.log >&2; fail "$bats_file: failed"; }
    fi
}

# Explore-trio worktree assert (issue #964, spec §D4): the autospec-explore
# trio (SKILL.md + codex/prompt.md + opencode/agent.md) must each carry the
# mandatory `worktree-guard.sh assert` call before any sandbox commit step,
# the MUST-exit-0 gate wording, the primary-checkout hard rule, and the
# Sandbox branch contract section must be byte-identical across all three
# files (lock-step). Runs tests/explore/test_explore_worktree_assert.bats
# when bats is on PATH.
check_explore_trio_worktree_assert() {
    info "explore-trio worktree assert (D4, issue #964)"
    for trio in \
        skills/autospec-explore/SKILL.md \
        skills/autospec-explore/codex/prompt.md \
        skills/autospec-explore/opencode/agent.md
    do
        [ -f "$trio" ] || fail "$trio: required adapter file missing"
        grep -q 'worktree-guard.sh assert' "$trio" \
            || fail "$trio: missing worktree-guard.sh assert before sandbox commit (D4)"
        grep -q 'MUST exit 0' "$trio" \
            || fail "$trio: assert gate must carry MUST-exit-0 wording (D4)"
        grep -qi 'primary checkout' "$trio" \
            || fail "$trio: missing primary-checkout hard rule (D4)"
    done
    # Lock-step: Sandbox branch contract section must be byte-identical.
    local s1 s2 s3
    s1="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' \
        skills/autospec-explore/SKILL.md)"
    s2="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' \
        skills/autospec-explore/codex/prompt.md)"
    s3="$(awk '/^## Sandbox branch contract/,/^## Research cycle contract/' \
        skills/autospec-explore/opencode/agent.md)"
    [ -n "$s1" ] || fail "SKILL.md: Sandbox branch contract section missing"
    diff <(printf '%s\n' "$s1") <(printf '%s\n' "$s2") \
        || fail "explore-trio: SKILL.md vs codex/prompt.md Sandbox branch contract differs (lock-step violation)"
    diff <(printf '%s\n' "$s1") <(printf '%s\n' "$s3") \
        || fail "explore-trio: SKILL.md vs opencode/agent.md Sandbox branch contract differs (lock-step violation)"
    local bats_file="tests/explore/test_explore_worktree_assert.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing (issue #964)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-explore-worktree-assert.log 2>&1 \
            || { cat /tmp/validate-explore-worktree-assert.log >&2; fail "$bats_file: failed"; }
    fi
    info "explore-trio worktree assert: all three adapter files carry D4 assert + lock-step verified"
}

# autospec-explore discovery enhancement contract (issues #1084/#1085, spec
# docs/specs/2026-06-15-autospec-explore-discovery-enhance.md §Acceptance):
# the discovery-quality batch (#1077–#1083) shipped 3 new researchers, the
# verify/ROI/synthesis/severity-first aggregator stages, the extended proposal
# + specialist schemas, and the domain-specialist roster. This gate asserts
# they are present + wired, the autospec-explore trio prose documents them in
# lock-step, and the runbook docs/runbooks/discovery-sweep.md lists the same
# five discovery tracks (A–E) as the spec. Runs the discovery bats suites when
# bats is on PATH.
check_autospec_explore_discovery_contract() {
    info "autospec-explore discovery enhancement contract (#1084/#1085)"

    # 1. The 3 discovery researchers exist, are executable, bash -n clean.
    for r in quality-resilience dogfooding self-leverage; do
        local script="scripts/explore-research/$r.sh"
        [ -f "$script" ] || fail "$script: required discovery researcher missing"
        [ -x "$script" ] || fail "$script: file not executable"
        bash -n "$script" || fail "$script: bash syntax error"
    done

    # 2. dogfooding reads the overridable state dir (portability/graceful-empty).
    grep -q 'AUTOSPEC_STATE_DIR' "scripts/explore-research/dogfooding.sh" \
        || fail "scripts/explore-research/dogfooding.sh: must read \${AUTOSPEC_STATE_DIR:-\$HOME/.autospec}"

    # 3. Aggregator stages: dedup -> verify -> ROI -> synthesis -> severity rank,
    #    plus the four new per-iteration log counters.
    local agg="scripts/explore-research-cycle.sh"
    [ -f "$agg" ] || fail "$agg: required aggregator missing"
    bash -n "$agg" || fail "$agg: bash syntax error"
    grep -q 'proposals_after_verify' "$agg" \
        || fail "$agg: missing verify-stage counter proposals_after_verify"
    grep -q 'proposals_refuted' "$agg" \
        || fail "$agg: missing proposals_refuted counter"
    grep -q 'proposals_after_roi' "$agg" \
        || fail "$agg: missing ROI-gate counter proposals_after_roi"
    grep -q 'structural_fixes' "$agg" \
        || fail "$agg: missing pattern-synthesis counter structural_fixes"
    grep -qi 'severity' "$agg" \
        || fail "$agg: missing severity-first ranking logic"

    # 4. The proposal + specialist schemas ship with the new fields.
    local pschema="schemas/autospec-explore-proposal.schema.json"
    local sschema="schemas/autospec-explore-specialists.schema.json"
    [ -f "$pschema" ] || fail "$pschema: required proposal schema missing"
    grep -q 'silent-wrong' "$pschema" \
        || fail "$pschema: severity enum must include silent-wrong (load-bearing rank head)"
    grep -q 'named_consumer' "$pschema" \
        || fail "$pschema: missing named_consumer ROI field"
    [ -f "$sschema" ] || fail "$sschema: required specialist schema missing"
    grep -q 'suggested_specialists' "$sschema" \
        || fail "$sschema: missing suggested_specialists roster field"
    [ -f "scripts/explore-specialist-scan.sh" ] \
        || fail "scripts/explore-specialist-scan.sh: required signal-scan missing"
    bash -n "scripts/explore-specialist-scan.sh" \
        || fail "scripts/explore-specialist-scan.sh: bash syntax error"

    # 5. Trio prose documents the enhancement in lock-step: the new sections,
    #    the 3 discovery researchers, the stage order, the severity enum, and
    #    the specialist flags must appear in all three adapter bodies. The
    #    stale "6 researcher" count must be gone.
    for trio in \
        skills/autospec-explore/SKILL.md \
        skills/autospec-explore/codex/prompt.md \
        skills/autospec-explore/opencode/agent.md
    do
        [ -f "$trio" ] || fail "$trio: required adapter file missing"
        grep -q '^## Discovery enhancement' "$trio" \
            || fail "$trio: missing '## Discovery enhancement' section"
        grep -q '^## Domain-specialist roster' "$trio" \
            || fail "$trio: missing '## Domain-specialist roster' section"
        for r in quality-resilience dogfooding self-leverage; do
            grep -q "$r" "$trio" \
                || fail "$trio: missing discovery researcher reference '$r'"
        done
        grep -q 'dedup . verify . ROI . pattern-synthesis . severity-first rank' "$trio" \
            || grep -q 'dedup -> verify -> ROI -> pattern-synthesis -> severity-first rank' "$trio" \
            || fail "$trio: missing aggregator stage order (dedup -> verify -> ROI -> pattern-synthesis -> severity-first rank)"
        for sev in silent-wrong correctness stability operability feature nicety; do
            grep -q "$sev" "$trio" \
                || fail "$trio: missing severity band '$sev'"
        done
        for flag in --specialists-mode --num-specialists; do
            grep -q -- "$flag" "$trio" \
                || fail "$trio: missing specialist flag '$flag'"
        done
        grep -q '7 universal' "$trio" \
            || fail "$trio: missing '7 universal' researcher accounting"
        if grep -Eq '\b6 researchers\b|each of the 6' "$trio"; then
            fail "$trio: stale '6 researchers' count still present (must be 7 universal)"
        fi
    done

    # 6. Runbook <-> spec five-track lock-step: both docs/runbooks/discovery-sweep.md
    #    and the spec must name the same five discovery tracks (A–E) plus the
    #    verify + synthesis steps.
    local runbook="docs/runbooks/discovery-sweep.md"
    local spec="docs/specs/2026-06-15-autospec-explore-discovery-enhance.md"
    [ -f "$runbook" ] || fail "$runbook: required runbook missing"
    [ -f "$spec" ] || fail "$spec: required source spec missing"
    for track in 'Feature delta' 'External' 'Quality & resilience' 'Dogfooding' 'Self-leverage'; do
        grep -q "$track" "$runbook" \
            || fail "$runbook: missing discovery track '$track' (runbook<->spec lock-step)"
        grep -q "$track" "$spec" \
            || fail "$spec: missing discovery track '$track' (runbook<->spec lock-step)"
    done
    grep -qi 'pattern synthesis' "$runbook" \
        || fail "$runbook: missing pattern-synthesis stage"
    grep -qi 'pattern synthesis' "$spec" \
        || fail "$spec: missing pattern-synthesis stage"
    # The runbook must cite this gate as the enforcing lock-step check.
    grep -q 'check_autospec_explore_discovery_contract' "$runbook" \
        || fail "$runbook: must cite check_autospec_explore_discovery_contract as the lock-step enforcer"

    # 7. Discovery bats suites run green when bats is available.
    for bats_file in \
        tests/explore/test_explore_quality_resilience.bats \
        tests/explore/test_explore_dogfooding.bats \
        tests/explore/test_explore_self_leverage.bats \
        tests/explore/test_explore_verify_stage.bats \
        tests/explore/test_explore_severity_roi.bats \
        tests/explore/test_explore_specialists.bats
    do
        [ -f "$bats_file" ] || fail "$bats_file: discovery bats coverage missing"
        if command -v bats >/dev/null 2>&1; then
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-explore-discovery.log 2>&1 \
                || { cat /tmp/validate-explore-discovery.log >&2; fail "$bats_file: failed"; }
        fi
    done

    info "autospec-explore discovery enhancement contract: researchers + stages + schemas + trio lock-step + runbook<->spec tracks verified"
}

# autospec-explore spec-first filing contract (issue #1102, spec
# docs/specs/2026-06-16-autospec-explore-spec-first-filing-design.md §Integration
# points + §Fallback): the perpetual loop must file each round's proposals
# spec-first — render a round design spec via gen-explore-round-spec.sh, commit +
# push it to the SANDBOX branch BEFORE decomposition, decompose via
# /autospec-define --base <sandbox> (never targeting main), and on a
# missing/failing define handoff log code_health:explore_define_unavailable,
# keep the committed spec, and fall back to raw filing. This gate asserts those
# wiring points are present in scripts/autospec-explore.sh and runs the two e2e
# bats suites when bats is on PATH.
check_autospec_explore_spec_first_contract() {
    info "autospec-explore spec-first filing contract (#1102)"
    local orch="scripts/autospec-explore.sh"
    [ -f "$orch" ] || fail "$orch: orchestrator missing"
    bash -n "$orch" || fail "$orch: bash syntax error"

    # 1. The round-spec renderer is invoked from the loop.
    grep -q 'gen-explore-round-spec\.sh' "$orch" \
        || fail "$orch: must invoke scripts/gen-explore-round-spec.sh to render the round spec (#1102)"

    # 2. The round-spec path convention is materialized under docs/specs/.
    grep -q 'docs/specs/.*explore-.*round-' "$orch" \
        || fail "$orch: must materialize docs/specs/<date>-explore-<slug>-round-<N>-design.md (#1102)"

    # 3. Commit-before-decompose ordering: the spec is git add + commit + pushed
    #    to the sandbox branch (HEAD:<sandbox>) before any decomposition.
    grep -q 'git commit' "$orch" \
        || fail "$orch: must commit the round spec before filing (#1102)"
    grep -q 'git push .*origin .*HEAD:\$SANDBOX_BRANCH' "$orch" \
        || fail "$orch: must push the round spec to the sandbox branch (HEAD:\$SANDBOX_BRANCH) before decompose (#1102)"

    # 4. Decomposition runs /autospec-define with --base <sandbox>, never main.
    grep -q '/autospec-define' "$orch" \
        || fail "$orch: round filing must decompose via /autospec-define (#1102)"
    grep -q -- '--base \$SANDBOX_BRANCH' "$orch" \
        || fail "$orch: decompose must pass --base \$SANDBOX_BRANCH (never main) (#1102)"

    # 5. Fallback identifier present + raw filing retained as the fallback path.
    grep -q 'code_health:explore_define_unavailable' "$orch" \
        || fail "$orch: missing fallback log token code_health:explore_define_unavailable (#1102)"
    grep -q '_explore_raw_file_round' "$orch" \
        || fail "$orch: must retain raw filing as the never-stall fallback (#1102)"

    # 6. e2e bats coverage exists + passes.
    for bats_file in \
        tests/explore/test_explore_spec_first_filing.bats \
        tests/explore/test_explore_define_fallback.bats
    do
        [ -f "$bats_file" ] || fail "$bats_file: spec-first bats coverage missing (#1102)"
        if command -v bats >/dev/null 2>&1; then
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-explore-spec-first.log 2>&1 \
                || { cat /tmp/validate-explore-spec-first.log >&2; fail "$bats_file: failed"; }
        fi
    done

    info "autospec-explore spec-first filing: renderer + sandbox-commit-before-decompose + --base + fallback verified"
}

# autospec-explore QA promotion gate loop integration (issue #1114): the
# orchestrator parses --qa-gate (default OFF) + --qa-gate-pass-on-partial, runs
# scripts/explore-qa-gate.sh ONCE at loop termination before the promotion
# block, branches the promotion-readiness output on the verdict, emits the two
# code_health tokens, and preserves a byte-unchanged default-off promotion path.
check_autospec_explore_qa_gate_contract() {
    info "autospec-explore QA promotion gate contract (#1114)"
    local orch="scripts/autospec-explore.sh"
    [ -f "$orch" ] || fail "$orch: orchestrator missing"
    bash -n "$orch" || fail "$orch: bash syntax error"

    # 1. Both flags are parsed.
    grep -q -- '--qa-gate)' "$orch" \
        || fail "$orch: must parse --qa-gate (#1114)"
    grep -q -- '--qa-gate-pass-on-partial)' "$orch" \
        || fail "$orch: must parse --qa-gate-pass-on-partial (#1114)"

    # 2. The runner from issue #1113 is invoked (guarded behind --qa-gate).
    grep -q 'explore-qa-gate\.sh' "$orch" \
        || fail "$orch: must invoke scripts/explore-qa-gate.sh (#1114)"

    # 3. Promotion-gating branches: PASS/skipped print the merge block; the
    #    fail-closed branch withholds it. Both promote+withhold paths must exist.
    grep -q 'To merge sandbox into main' "$orch" \
        || fail "$orch: must retain the merge-instructions promotion block (#1114)"
    grep -q 'Promotion WITHHELD' "$orch" \
        || fail "$orch: must withhold the merge block on a blocking verdict (#1114)"
    grep -q 'sandbox QA:' "$orch" \
        || fail "$orch: must annotate the promotion output with the sandbox QA verdict (#1114)"
    grep -q 'no QA config' "$orch" \
        || fail "$orch: skipped verdict must carry the no-config annotation (#1114)"

    # 4. The two code_health tokens (gate ran inert/failed + the runner's skip).
    grep -q 'code_health:explore_qa_gate_failed' "$orch" \
        || fail "$orch: must emit code_health:explore_qa_gate_failed on a blocking verdict (#1114)"
    grep -q 'code_health:explore_qa_gate_skipped_no_config' scripts/explore-qa-gate.sh \
        || fail "scripts/explore-qa-gate.sh: must emit code_health:explore_qa_gate_skipped_no_config (#1113/#1114)"

    # 5. Staleness warning: compares the current sandbox tip to the gate sha.
    grep -q 'sandbox_head_sha' "$orch" \
        || fail "$orch: must warn when the sandbox advances past the gate sandbox_head_sha (#1114)"

    # 6. Default-off byte-unchanged path: the promotion block is guarded so the
    #    ungated path emits the legacy output verbatim.
    grep -q 'QA_GATE.*-eq 0' "$orch" \
        || fail "$orch: must guard the default-off byte-unchanged promotion path (#1114)"

    # 7. The gate verdict row is recorded in the loop json.
    grep -q 'qa_gate_verdict' "$orch" \
        || fail "$orch: must record the gate verdict in .autospec/explore-loop.json (#1114)"

    # 8. bats coverage exists + passes.
    local bats_file="tests/explore/test_explore_qa_gate_promotion.bats"
    [ -f "$bats_file" ] || fail "$bats_file: QA-gate promotion bats coverage missing (#1114)"
    # Cross-script composition guard (#1116): the real runner -> real loop path
    # (no AUTOSPEC_EXPLORE_QA_GATE_CMD seam) — catches a runner/loop field or
    # path rename that the two isolated suites would both miss.
    local e2e_file="tests/explore/test_explore_qa_gate_e2e.bats"
    [ -f "$e2e_file" ] || fail "$e2e_file: QA-gate cross-script e2e coverage missing (#1116)"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-explore-qa-gate.log 2>&1 \
            || { cat /tmp/validate-explore-qa-gate.log >&2; fail "$bats_file: failed"; }
        info "  running: $e2e_file"
        bats "$e2e_file" >/tmp/validate-explore-qa-gate-e2e.log 2>&1 \
            || { cat /tmp/validate-explore-qa-gate-e2e.log >&2; fail "$e2e_file: failed"; }
    fi

    info "autospec-explore QA promotion gate: flags + single-run gate + verdict-gated promotion + code_health + default-off + cross-script e2e verified"
}

# Release area-dispatch contract (issue #731): 6 area files exist under
# skills/autospec-release/areas/, scripts/release-area-dispatch.sh dispatches
# them in parallel via the harness-aware dispatcher (PR #725), trio prose
# references the new area directory, and the bats fixture asserts the
# aggregated verdict shape that compute-release-verdict.sh (PR #636) consumes.
check_autospec_release_area_contract() {
    info "autospec-release area dispatch: 6 areas + dispatcher + trio lockstep"
    local areas_dir="skills/autospec-release/areas"
    [ -d "$areas_dir" ] || fail "$areas_dir: directory missing (issue #731)"
    for area in spec-completeness docs-freshness implementation-completeness \
                test-coverage qa-artifact-integrity legacy-cleanup; do
        [ -f "$areas_dir/$area.md" ] \
            || fail "$areas_dir/$area.md: required area file missing (issue #731)"
    done
    local script="scripts/release-area-dispatch.sh"
    [ -f "$script" ] || fail "$script: required (issue #731)"
    [ -x "$script" ] || fail "$script: must be executable (issue #731)"
    bash -n "$script" || fail "$script: bash syntax error (issue #731)"
    for trio in skills/autospec-release/SKILL.md \
                skills/autospec-release/codex/prompt.md \
                skills/autospec-release/opencode/agent.md; do
        grep -q 'release-area-dispatch\.sh' "$trio" \
            || fail "$trio missing reference to scripts/release-area-dispatch.sh (issue #731)"
    done
    if command -v bats >/dev/null 2>&1 && [ -f tests/release/test_release_area_dispatch.bats ]; then
        info "  running: tests/release/test_release_area_dispatch.bats"
        bats tests/release/test_release_area_dispatch.bats \
            >/tmp/validate-release-area-dispatch.log 2>&1 \
            || { cat /tmp/validate-release-area-dispatch.log >&2; fail "tests/release/test_release_area_dispatch.bats: failed"; }
    fi
}

# Release-trio worktree assert gate (issue #965, spec §D4): every file in the
# autospec-release trio must carry a `## Worktree guard (commit gate)` section
# with `worktree-guard.sh assert` before any commit, the MANDATORY/MUST-exit-0
# wording, a `worktree-assert:begin/end` sentinel block, and the primary-checkout
# and cleanup rules. Lock-step: the section must be byte-identical across the
# trio (SKILL.md == codex/prompt.md == opencode/agent.md prose block).
check_release_trio_worktree_assert() {
    info "release-trio worktree assert gate (D4, issue #965)"
    local trio="skills/autospec-release/SKILL.md \
                skills/autospec-release/codex/prompt.md \
                skills/autospec-release/opencode/agent.md"
    for f in $trio; do
        [ -f "$f" ] || fail "$f: required file missing (issue #965)"
        grep -q '## Worktree guard (commit gate)' "$f" \
            || fail "$f: missing '## Worktree guard (commit gate)' section (issue #965)"
        grep -q 'worktree-guard.sh assert' "$f" \
            || fail "$f: missing worktree-guard.sh assert call (issue #965)"
        grep -q 'MUST exit 0' "$f" \
            || fail "$f: assert gate must state MUST exit 0 (issue #965)"
        grep -q 'worktree-assert:begin' "$f" \
            || fail "$f: missing worktree-assert:begin sentinel (issue #965)"
        grep -q 'worktree-assert:end' "$f" \
            || fail "$f: missing worktree-assert:end sentinel (issue #965)"
        grep -qi 'primary checkout' "$f" \
            || fail "$f: missing primary-checkout hard rule (issue #965)"
        grep -q 'git worktree remove' "$f" \
            || fail "$f: missing 'git worktree remove' cleanup rule (issue #965)"
        grep -q 'git worktree prune' "$f" \
            || fail "$f: missing 'git worktree prune' cleanup rule (issue #965)"
    done
    # Lock-step: the prose block between sentinels must be byte-identical across trio.
    local skill_block codex_block opencode_block
    skill_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' \
        skills/autospec-release/SKILL.md)"
    codex_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' \
        skills/autospec-release/codex/prompt.md)"
    opencode_block="$(awk '/<!-- worktree-assert:begin -->/,/<!-- worktree-assert:end -->/' \
        skills/autospec-release/opencode/agent.md)"
    [ "$skill_block" = "$codex_block" ] \
        || fail "SKILL.md and codex/prompt.md worktree-assert blocks differ (lock-step violation, issue #965)"
    [ "$skill_block" = "$opencode_block" ] \
        || fail "SKILL.md and opencode/agent.md worktree-assert blocks differ (lock-step violation, issue #965)"
    if command -v bats >/dev/null 2>&1 && [ -f tests/release/test_release_worktree_assert.bats ]; then
        info "  running: tests/release/test_release_worktree_assert.bats"
        bats tests/release/test_release_worktree_assert.bats \
            >/tmp/validate-release-worktree-assert.log 2>&1 \
            || { cat /tmp/validate-release-worktree-assert.log >&2; fail "tests/release/test_release_worktree_assert.bats: failed"; }
    fi
    info "release-trio worktree assert gate: all 3 trio files carry D4 assert"
}

# Crash-resume skill + durable run registry contract (issue #881):
# the new autospec-resume skill ships the trio + scaffolding + per-skill
# validate.sh; resume-scan.sh / resume-attempts.sh / autospec-run-registry.sh
# exist, are executable, and bash -n clean; the heartbeat schema carries a
# backward-compatible `host` field; and /autospec-run wires the registry write
# in lockstep across its trio. Runs the per-skill validate.sh and the
# tests/resume bats fixtures when bats is on PATH.
# Files + structural sections + script syntax for the autospec-resume skill.
check_autospec_resume_structure() {
    local skill="skills/autospec-resume"
    [ -d "$skill" ] || fail "$skill: skill directory missing (issue #881)"
    for f in SKILL.md README.md install.sh uninstall.sh validate.sh \
             opencode/agent.md codex/prompt.md \
             scripts/resume-scan.sh scripts/resume-attempts.sh; do
        [ -f "$skill/$f" ] || fail "$skill/$f: required file missing (issue #881)"
    done
    # Transition-safe (D3a sweep, #1032/#1035): the canonical '## Startup
    # self-update' section was replaced by the
    # <!-- autospec-block:startup-self-update SKILL_NAME=autospec-resume -->
    # marker. Accept EITHER form; require SKILL_NAME=autospec-resume in whichever
    # form is present (the marker carries it inline). Files with NEITHER fail.
    if grep -q '^## Startup self-update' "$skill/SKILL.md"; then
        grep -q 'SKILL_NAME=autospec-resume' "$skill/SKILL.md" \
            || fail "$skill/SKILL.md Startup self-update missing SKILL_NAME=autospec-resume"
    elif grep -q 'autospec-block:startup-self-update' "$skill/SKILL.md"; then
        grep -q 'autospec-block:startup-self-update SKILL_NAME=autospec-resume' "$skill/SKILL.md" \
            || fail "$skill/SKILL.md startup-self-update marker missing SKILL_NAME=autospec-resume"
    else
        fail "$skill/SKILL.md missing ## Startup self-update section or autospec-block:startup-self-update marker"
    fi
    grep -q '^## Harness detection' "$skill/SKILL.md" \
        || fail "$skill/SKILL.md missing harness-detection section"
    grep -q '^## Required capabilities & harness adapter' "$skill/SKILL.md" \
        || fail "$skill/SKILL.md missing ## Required capabilities & harness adapter row"
    grep -q 'Subagent model tier' "$skill/SKILL.md" \
        || fail "$skill/SKILL.md adapter table missing 'Subagent model tier' row"
    for s in "$skill/scripts/resume-scan.sh" \
             "$skill/scripts/resume-attempts.sh" \
             "scripts/autospec-run-registry.sh"; do
        [ -f "$s" ] || fail "$s: required script missing (issue #881)"
        [ -x "$s" ] || fail "$s: file not executable (issue #881)"
        bash -n "$s" || fail "$s: bash syntax error (issue #881)"
    done
}

# External boot supervisor (issue #882, child 2 of the crash-resume spec): the
# boot entrypoint + cross-platform install surface must exist, be executable,
# and pass `bash -n`. The supervisor must delegate relaunch to /autospec-resume
# (resume-scan.sh) and never eval the registry's raw resume_command directly,
# and the installer must select launchd / systemd / @reboot cron by platform.
check_autospec_supervisor_structure() {
    info "autospec-supervisor boot supervisor + install surface (issue #882)"
    for s in scripts/autospec-supervisor.sh scripts/autospec-supervisor-install.sh; do
        [ -f "$s" ] || fail "$s: required script missing (issue #882)"
        [ -x "$s" ] || fail "$s: file not executable (issue #882)"
        bash -n "$s" || fail "$s: bash syntax error (issue #882)"
    done
    # Security: the supervisor relaunches only via /autospec-resume (resume-scan),
    # never by eval'ing the registry's raw resume_command.
    grep -q 'resume-scan.sh' scripts/autospec-supervisor.sh \
        || fail "scripts/autospec-supervisor.sh must delegate relaunch to resume-scan.sh (/autospec-resume)"
    if grep -Eq 'eval[[:space:]]+"\$(resume_command|reg)' scripts/autospec-supervisor.sh; then
        fail "scripts/autospec-supervisor.sh must NOT eval the registry's raw resume_command directly (supervisor safety)"
    fi
    grep -q 'confirm_open_run\|in-progress-by-bot' scripts/autospec-supervisor.sh \
        || fail "scripts/autospec-supervisor.sh must independently confirm an open in-progress run via gh"
    # Installer must cover all three platform surfaces.
    for needle in launchd systemd '@reboot'; do
        grep -q "$needle" scripts/autospec-supervisor-install.sh \
            || fail "scripts/autospec-supervisor-install.sh missing $needle platform surface (issue #882)"
    done
    for action in install uninstall status; do
        grep -q "^[[:space:]]*$action)" scripts/autospec-supervisor-install.sh \
            || fail "scripts/autospec-supervisor-install.sh missing '$action' action (issue #882)"
    done
}

# Crash-resume invariants: no second lock, server updated_at, host field,
# registry wiring, per-skill lint, and bats coverage.
check_autospec_resume_contract() {
    info "autospec-resume crash-resume skill + durable run registry (issue #881)"
    local skill="skills/autospec-resume"
    check_autospec_resume_structure
    check_autospec_supervisor_structure

    # Idempotency / no-second-lock: resume-scan reads run-state but never
    # upserts/clears it and never writes a run-state comment.
    if grep -E 'run-state\.sh[^[:alnum:]]*upsert|run-state\.sh[^[:alnum:]]*clear|gh issue comment' \
            "$skill/scripts/resume-scan.sh" >/dev/null; then
        fail "$skill/scripts/resume-scan.sh must add no second lock (found a run-state mutation)"
    fi
    grep -q 'updated_at' "$skill/scripts/resume-scan.sh" \
        || fail "$skill/scripts/resume-scan.sh must use run-state server updated_at"
    grep -q '"host"' "skills/autospec-run/scripts/heartbeat-write.sh" \
        || fail "heartbeat-write.sh must write a backward-compatible host field (issue #881)"
    for trio in skills/autospec-run/SKILL.md \
                skills/autospec-run/codex/prompt.md \
                skills/autospec-run/opencode/agent.md; do
        grep -q 'autospec-run-registry.sh' "$trio" \
            || fail "$trio missing autospec-run-registry.sh registry-write wiring (issue #881)"
    done

    info "  running: $skill/validate.sh"
    bash "$skill/validate.sh" >/tmp/validate-resume-skill.log 2>&1 \
        || { cat /tmp/validate-resume-skill.log >&2; fail "$skill/validate.sh: failed"; }

    [ -d tests/resume ] || fail "tests/resume: bats coverage directory missing (issue #881)"
    if command -v bats >/dev/null 2>&1; then
        for bats_file in tests/resume/*.bats; do
            [ -f "$bats_file" ] || continue
            info "  running: $bats_file"
            bats "$bats_file" >/tmp/validate-resume-bats.log 2>&1 \
                || { cat /tmp/validate-resume-bats.log >&2; fail "$bats_file: failed"; }
        done
    fi
}

# Palette single-source check (issue #920): the 6 light-blue hex constants must
# live ONLY in skills/autospec-doc/scripts/doc-style.mjs.  No other .mjs, .sh,
# or .md file in the repo may hardcode any of those hex values.
check_palette_single_source() {
    info "palette single-source: 6 light-blue hexes must live only in doc-style.mjs (issue #920)"
    local doc_style="skills/autospec-doc/scripts/doc-style.mjs"
    local doc_style_test="skills/autospec-doc/tests/doc-style.test.mjs"
    [ -f "$doc_style" ] || { fail "$doc_style: file missing"; return; }

    # Read the 6 pinned hex values directly from the source of truth.
    local hexes
    hexes=$(grep -oE '#[0-9A-Fa-f]{6}' "$doc_style" | sort -u)
    if [ -z "$hexes" ]; then
        fail "$doc_style: no hex values found — palette single-source check cannot run"
        return
    fi

    local violations=()
    while IFS= read -r hex; do
        # Search all .mjs and .sh files (NOT .md — specs and docs are allowed to
        # document the hex values; only source code must not hardcode them).
        # Exclude doc-style.mjs itself and its test file.
        while IFS= read -r hit; do
            # Normalize path (strip leading ./)
            local norm_hit="${hit#./}"
            [ "$norm_hit" = "$doc_style" ]      && continue
            [ "$norm_hit" = "$doc_style_test" ] && continue
            violations+=("$norm_hit contains palette hex $hex")
        done < <(grep -rl "$hex" --include="*.mjs" --include="*.sh" . 2>/dev/null || true)
    done <<< "$hexes"

    if [ "${#violations[@]}" -gt 0 ]; then
        for v in "${violations[@]}"; do
            fail "palette single-source violation: $v"
        done
    fi
}

# autospec-doc scaffold contract (issue #916): the per-audience documentation
# generator trio + subcommand router stub. Enforces:
#   - all three trio files carry a `## Subcommand contract` section documenting
#     the five §D1 forms (bare incremental, --full, --audit, --audience <name>,
#     init), so the contract stays in lock-step across harnesses;
#   - scripts/doc-orchestrator.mjs exists, node --check clean, and exits 2 on a
#     non-init subcommand with no `documentation:` config;
#   - the bats suite exists and passes.
# Generic trio checks (lock-step, frontmatter, self-update, model-tier,
# harness-detection, required-files) run via the discover_skills loop in main().
check_autospec_doc_contract() {
    info "autospec-doc scaffold contract (issue #916)"
    local skill="skills/autospec-doc"
    [ -d "$skill" ] || fail "$skill: directory missing"
    for trio in SKILL.md codex/prompt.md opencode/agent.md; do
        f="$skill/$trio"
        [ -f "$f" ] || fail "$f: required file missing"
        grep -q '^## Subcommand contract' "$f" \
            || fail "$f missing '## Subcommand contract' section"
        # The five §D1 subcommand forms must all be documented in every trio file.
        grep -q -- '--full' "$f"     || fail "$f missing --full subcommand"
        grep -q -- '--audit' "$f"    || fail "$f missing --audit subcommand"
        grep -q -- '--audience' "$f" || fail "$f missing --audience subcommand"
        grep -qE '/autospec-doc init|`init`' "$f" \
            || fail "$f missing init subcommand"
        # §D4 worktree assert: the regenerate commit step must be preceded by a
        # worktree-guard.sh assert call. The sentinel block must be present and
        # the assert MUST exit 0 rule must be stated in every trio file.
        grep -q 'worktree-guard.sh assert' "$f" \
            || fail "$f: missing worktree-guard.sh assert before doc regenerate commit (issue #963 §D4)"
        grep -q 'MUST exit 0' "$f" \
            || fail "$f: doc-regen assert gate must require exit 0 before commit (issue #963 §D4)"
        grep -q 'doc-regen-assert:begin' "$f" \
            || fail "$f: missing doc-regen-assert:begin sentinel (issue #963 §D4)"
    done
    orch="$skill/scripts/doc-orchestrator.mjs"
    [ -f "$orch" ] || fail "$orch: orchestrator stub missing"
    if command -v node >/dev/null 2>&1; then
        node --check "$orch" || fail "$orch: node --check failed"
        # Config gate: a non-init subcommand with no documentation config exits 2.
        # Capture the exit without tripping `set -e` (the non-zero exit is expected).
        gate_dir="$(mktemp -d)"
        rc=0
        ( cd "$gate_dir" && node "$REPO_ROOT/$orch" --full ) >/dev/null 2>&1 || rc=$?
        rm -rf "$gate_dir"
        [ "$rc" -eq 2 ] \
            || fail "$orch: non-init subcommand with no documentation config must exit 2 (got $rc)"
    fi
    bats_file="$skill/tests/doc-orchestrator.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing"
    if command -v bats >/dev/null 2>&1; then
        info "  running: $bats_file"
        bats "$bats_file" >/tmp/validate-autospec-doc-bats.log 2>&1 \
            || { cat /tmp/validate-autospec-doc-bats.log >&2; fail "$bats_file: failed"; }
    fi
}

# Issue #956: QA documentation gate (D3).
# Enforces that the '## Documentation gate' section is present byte-identically
# across the autospec-qa trio (SKILL.md + codex/prompt.md + opencode/agent.md),
# and that the bats coverage file exists.
check_qa_documentation_gate() {
    info "qa documentation gate lockstep: autospec-qa trio (issue #956)"
    local skill="skills/autospec-qa"
    for trio in SKILL.md codex/prompt.md opencode/agent.md; do
        local f="$skill/$trio"
        [ -f "$f" ] || fail "$f: required file missing"
        grep -q '^## Documentation gate' "$f" \
            || fail "$f: missing '## Documentation gate' section (issue #956)"
        grep -q 'doc-orchestrator.mjs --full' "$f" \
            || fail "$f: Documentation gate must reference doc-orchestrator.mjs --full"
        grep -q 'verify-examples.mjs' "$f" \
            || fail "$f: Documentation gate must reference verify-examples.mjs"
        grep -q 'check-doc-drift.sh --working-tree' "$f" \
            || fail "$f: Documentation gate must reference check-doc-drift.sh --working-tree"
        grep -q 'auto_regenerate.*NOT consulted\|NOT consulted.*auto_regenerate\|auto_regenerate` is NOT consulted' "$f" \
            || fail "$f: Documentation gate must explicitly state auto_regenerate is NOT consulted"
        grep -q 'orchestrator exits 2\|exit 2\|exits 2' "$f" \
            || fail "$f: Documentation gate must document graceful skip on orchestrator exit 2"
    done
    local bats_file="tests/qa/test_qa_documentation_gate.bats"
    [ -f "$bats_file" ] || fail "$bats_file: bats coverage missing (issue #956)"
}

# claim-guard contract (issue #1066): scripts/claim-guard.sh must be present
# and executable, pass `bash -n` syntax check, and the overlap bats suite must
# be green. Mirrors the pattern used by check_supersession_contract and
# check_watchdog_worktree_gc.
check_claim_guard_contract() {
    info "claim-guard contract: scripts/claim-guard.sh + bats suite (issue #1066)"
    local guard="scripts/claim-guard.sh"
    [ -f "$guard" ] \
        || fail "$guard: file missing (issue #1066)"
    [ -x "$guard" ] \
        || fail "$guard: file not executable (issue #1066)"
    bash -n "$guard" \
        || fail "$guard: bash syntax error (issue #1066)"
    # scan subcommand must be wired up (the core contract of issue #1066).
    grep -q '^        scan)' "$guard" \
        || fail "$guard: 'scan' subcommand missing from dispatch (issue #1066)"
    # Spec-named suites (docs/specs/2026-06-15-claim-guard-concurrent-edit-design.md
    # §"Testing"): overlap, plus the stale-reclaim boundary and degrade-to-no-op
    # suites the Phase 5.5 audit (issue #1069) made first-class so they cannot
    # silently rot.
    local bats_files="tests/test_claim_guard_overlap.bats tests/test_claim_guard_stale_reclaim.bats tests/test_claim_guard_degrade.bats"
    local bf
    for bf in $bats_files; do
        [ -f "$bf" ] \
            || fail "$bf: bats coverage missing (issue #1066/#1069)"
    done
    if command -v bats >/dev/null 2>&1; then
        for bf in $bats_files; do
            info "  running: $bf"
            bats "$bf" >/tmp/validate-claim-guard.log 2>&1 \
                || { cat /tmp/validate-claim-guard.log >&2; \
                     fail "$bf: failed (issue #1066/#1069)"; }
        done
    fi
}

main "$@"
