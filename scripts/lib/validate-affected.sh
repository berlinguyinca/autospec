#!/usr/bin/env bash
# scripts/lib/validate-affected.sh — affected-set mapping for the scoped
# `--changed`/`--since` fast path in scripts/validate.sh (issue #1122).
#
# Single source of truth for "given a changed-file set, which validation checks
# must run?". Sourced by scripts/validate.sh main() when `--changed`/`--since`
# is passed. Bare (no-flag) validate.sh never sources/uses this — its default
# full-serial path stays byte-for-byte unchanged (the merge gate).
#
# Design — FAIL SAFE. When in doubt, RUN the check:
#   1. ALWAYS-RUN trigger: if the changed set touches any *shared input*
#      (AGENTS.md, scripts/lib/**, scripts/validate.sh, scripts/expand-skill-blocks.sh),
#      every check runs — `--changed` degrades to (close to) the full run. This
#      guarantees `--changed` NEVER silently skips a check whose correctness
#      could depend on a shared input.
#   2. Per-skill scoping: a skill's checks (lock-step, goldens, frontmatter,
#      required-files, self-update, model-tier, harness-block, monitor-exit) run
#      iff a changed file is under skills/<name>/ OR is that skill's golden
#      (tests/fixtures/skill-goldens/<name>.*) — OR rule (1) fired.
#   3. Unmapped default: any check not explicitly mapped to a narrower input set
#      defaults to RUN. New checks are safe by construction.
#
# All matching is plain prefix/substring over a real changed-file list file
# (one path per line). macOS bash 3.2 safe: no `[ -f <(...) ]`, no RETURN traps,
# if/then/fi for one-sided conditionals (no `[ ] && action` under set -e).

# Guard double-source.
if [ -n "${_VALIDATE_AFFECTED_LOADED:-}" ]; then
    return 0 2>/dev/null || true
fi
_VALIDATE_AFFECTED_LOADED=1

# Shared inputs whose change forces a (near) full run. A changed path is "shared"
# if it is exactly AGENTS.md / scripts/validate.sh / scripts/expand-skill-blocks.sh,
# or lives under scripts/lib/.
#
# Returns 0 (shared → degrade to full) if ANY line of the changed-file list file
# matches a shared input; 1 otherwise. Empty/missing list → 1 (not shared).
validate_affected_shared_changed() {
    changed_file="$1"
    [ -f "$changed_file" ] || return 1
    while IFS= read -r path || [ -n "$path" ]; do
        [ -n "$path" ] || continue
        case "$path" in
            AGENTS.md) return 0 ;;
            scripts/validate.sh) return 0 ;;
            scripts/expand-skill-blocks.sh) return 0 ;;
            scripts/lib/*) return 0 ;;
        esac
    done < "$changed_file"
    return 1
}

# Per-skill scoping decision. Returns 0 (run this skill's checks) if a shared
# input changed (degrade to full) OR a changed file belongs to this skill
# (skills/<name>/** or tests/fixtures/skill-goldens/<name>.*); 1 otherwise.
validate_affected_skill_runs() {
    skill_name="$1"
    changed_file="$2"
    if validate_affected_shared_changed "$changed_file"; then
        return 0
    fi
    [ -f "$changed_file" ] || return 0
    while IFS= read -r path || [ -n "$path" ]; do
        [ -n "$path" ] || continue
        case "$path" in
            "skills/$skill_name/"*) return 0 ;;
            "tests/fixtures/skill-goldens/$skill_name."*) return 0 ;;
        esac
    done < "$changed_file"
    return 1
}

# Global (non-per-skill) check scoping decision. Fail-safe: unmapped checks
# default to RUN. A shared-input change forces RUN. Today every global check is
# unmapped, so this always returns 0; the hook exists so a future change can map
# a specific global check to a narrower input set without touching the driver.
# Returns 0 (run) / 1 (skip).
validate_affected_check_runs() {
    check_name="$1"
    changed_file="$2"
    if validate_affected_shared_changed "$changed_file"; then
        return 0
    fi
    # No narrow mappings declared yet → default RUN (fail safe).
    case "$check_name" in
        *) return 0 ;;
    esac
}

# Spec-named convenience: emit the set of per-skill names whose checks should run
# given the changed-file list, drawn from the candidate skill-name list passed on
# stdin (one name per line). Used by callers that want the affected set directly.
# A shared-input change emits every candidate (degrade to full).
affected_checks() {
    changed_file="$1"
    while IFS= read -r skill_name || [ -n "$skill_name" ]; do
        [ -n "$skill_name" ] || continue
        if validate_affected_skill_runs "$skill_name" "$changed_file"; then
            printf '%s\n' "$skill_name"
        fi
    done
}
