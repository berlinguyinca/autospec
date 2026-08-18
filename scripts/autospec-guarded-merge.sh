#!/usr/bin/env bash
# autospec-guarded-merge.sh — fused blast-radius domain fence + admin merge.
#
# Runs the blast-radius classifier on a PR's ACTUAL changed files immediately
# before merging. If the diff touches a configured fenced surface and no
# override label is present, it quarantines the PR (applies the human-review
# label, comments the fenced surfaces, and refuses to merge). Otherwise it
# performs the admin squash-merge.
#
# This is the merge-time, per-diff enforcement point that the predictive
# selection-time fence (autonomous-prioritize.sh) and the tier-level premerge
# gate do not cover. Callers in /autospec-run invoke this INSTEAD of a bare
# `gh pr merge --admin`, so "merge without the fence check" requires
# deliberately bypassing the wrapper rather than merely omitting a prose step.
#
# NOTE: this is implementer-honored (soft) — the genuinely unbypassable fence
# is branch protection + a required status check (CI). This wrapper is the
# strongest per-diff fence available without CI.
#
# Usage:
#   autospec-guarded-merge.sh --pr N --repo OWNER/REPO
#       [--fenced-surfaces FILE]     # default: guardrails resolves .autospec/*
#       [--override-label LABEL]     # default: autospec:fenced-approved
#       [--human-label LABEL]        # default: autospec:needs-human
#       [--merge-args "ARGS"]        # default: --admin --squash --delete-branch
#       [--no-require-checks]        # skip the CI-conclusion gate (default: on)
#       [--checks-timeout SECS]      # default: 1800
#       [--checks-poll SECS]         # default: 30
#
# CI-conclusion gate (issue #3220). `main` carries no branch protection, so no
# check is "required" and a PR whose checks are pending or failing reports
# mergeStateStatus UNSTABLE. The Phase 4 loop treats UNSTABLE as ready and
# breaks straight to merge, so its `wait_for_ci_green` never runs on the normal
# path — #3148 and #3216 both merged before their run reported, and #3148's
# `build-test` was already failing. Enforcing it here, at the chokepoint every
# auto-implement merge routes through, makes "merge before CI reports" require
# deliberately passing --no-require-checks rather than merely reaching the merge
# by a path whose prose forgot to wait.
#
# Advisory checks are operator-declared via AUTOSPEC_PR_ADVISORY_CHECKS,
# defaulting to AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS — the same regex the
# conductor main-health gate honors, so there is one definition, not two.
#
# Exit codes:
#   0  merged (allowed, or fenced-but-overridden)
#   1  refused — NOT merged (fenced surface without override, or non-advisory
#      checks not green)
#   2  invocation / classifier error — fail-closed, NOT merged
#
# Engineering rules (AGENTS.md): set -euo pipefail; if/then/fi (no one-sided
# && short-circuits); no RETURN traps (inline cleanup).

set -euo pipefail

PR=""
REPO=""
FENCED_SURFACES=""
OVERRIDE_LABEL="autospec:fenced-approved"
HUMAN_LABEL="autospec:needs-human"
MERGE_ARGS="--admin --squash --delete-branch"
REQUIRE_CHECKS=1
CHECKS_TIMEOUT=1800
CHECKS_POLL=30

_die() {
    printf 'autospec-guarded-merge: %s\n' "$1" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr) PR="${2:-}"; shift 2 ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        --fenced-surfaces) FENCED_SURFACES="${2:-}"; shift 2 ;;
        --override-label) OVERRIDE_LABEL="${2:-}"; shift 2 ;;
        --human-label) HUMAN_LABEL="${2:-}"; shift 2 ;;
        --merge-args) MERGE_ARGS="${2:-}"; shift 2 ;;
        --no-require-checks) REQUIRE_CHECKS=0; shift 1 ;;
        --checks-timeout) CHECKS_TIMEOUT="${2:-}"; shift 2 ;;
        --checks-poll) CHECKS_POLL="${2:-}"; shift 2 ;;
        -h|--help) sed -n 's/^# \?//p' "$0" | head -40; exit 0 ;;
        *) _die "unknown option: $1" ;;
    esac
done

[ -n "$PR" ] || _die "--pr is required"
[ -n "$REPO" ] || _die "--repo is required"

# Resolve the guardrails helper (sibling of this script; test override allowed).
_GUARDRAILS="${AUTOSPEC_GUARDRAILS_SH:-}"
if [ -z "$_GUARDRAILS" ]; then
    _dir="$(cd "$(dirname "$0")" && pwd)"
    _GUARDRAILS="$_dir/autonomous-guardrails.sh"
fi
[ -f "$_GUARDRAILS" ] || _die "guardrails helper not found: $_GUARDRAILS"

_TMPDIR="$(mktemp -d -t guarded-merge.XXXXXX)"
_cleanup() { rm -rf "$_TMPDIR" 2>/dev/null || true; }

# 1. Fetch the PR's actual changed files. gh failure is fail-closed.
_changed="$_TMPDIR/changed.txt"
if ! gh pr view "$PR" --repo "$REPO" --json files \
        --jq '.files[].path' > "$_changed" 2>/dev/null; then
    _cleanup
    _die "could not read changed files for PR #$PR (fail-closed, not merged)"
fi

# Empty diff → nothing to classify → allow (proceed to merge).
if [ ! -s "$_changed" ]; then
    printf 'guarded-merge: PR #%s has no changed files; nothing fenced\n' "$PR"
else
    # 2. Classify blast radius against the fenced-surfaces registry.
    _blast_out=""
    _blast_rc=0
    if [ -n "$FENCED_SURFACES" ]; then
        _blast_out="$(bash "$_GUARDRAILS" blast-radius --changed-files "$_changed" \
            --fenced-surfaces "$FENCED_SURFACES" 2>&1)" || _blast_rc=$?
    else
        _blast_out="$(bash "$_GUARDRAILS" blast-radius --changed-files "$_changed" \
            2>&1)" || _blast_rc=$?
    fi

    # 3. Branch on the deterministic DECISION line (not exit code alone), so a
    #    classifier error (no DECISION line) is distinguished from a real
    #    quarantine and fails closed.
    if printf '%s\n' "$_blast_out" | grep -q '^DECISION:quarantine'; then
        # Fenced. Honor an explicit override label on the PR.
        _labels="$(gh pr view "$PR" --repo "$REPO" --json labels \
            --jq '.labels[].name' 2>/dev/null || true)"
        if printf '%s\n' "$_labels" | grep -qxF "$OVERRIDE_LABEL"; then
            printf 'guarded-merge: PR #%s touches a fenced surface but carries override label %s; proceeding\n' \
                "$PR" "$OVERRIDE_LABEL"
        else
            _surfaces="$(printf '%s\n' "$_blast_out" | grep '^SURFACE:' || true)"
            gh pr edit "$PR" --repo "$REPO" --add-label "$HUMAN_LABEL" >/dev/null 2>&1 || true
            gh pr comment "$PR" --repo "$REPO" --body "$(printf 'Blocked by the blast-radius domain fence: this PR touches a fenced surface and requires human review before merge.\n\n```\n%s\n```\n\nAdd the `%s` label after review to override.' "${_surfaces:-$_blast_out}" "$OVERRIDE_LABEL")" >/dev/null 2>&1 || true
            printf '%s\n' "$_blast_out"
            printf 'blocked fenced_surface\n'
            _cleanup
            exit 1
        fi
    elif printf '%s\n' "$_blast_out" | grep -q '^DECISION:allow'; then
        printf 'guarded-merge: PR #%s blast-radius allowed\n' "$PR"
    else
        # No parseable DECISION — classifier error. Fail closed.
        printf '%s\n' "$_blast_out" >&2
        _cleanup
        _die "blast-radius classifier produced no DECISION for PR #$PR (fail-closed, not merged)"
    fi
fi

# 4. CI-conclusion gate: refuse while any non-advisory check is pending or not
#    green. A null conclusion means "still running"; counting it as success is
#    exactly how a merge races its own CI run.
_ADVISORY="${AUTOSPEC_PR_ADVISORY_CHECKS:-${AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS:-^$}}"

_rollup_counts() {
    # Emits "<pending> <bad> <total>" for non-advisory entries. A CheckRun
    # carries .conclusion; a StatusContext carries .state — honor both, or a
    # legacy status context reads as pending forever and stalls every merge.
    printf '%s' "$1" | jq -r --arg adv "$_ADVISORY" '
        [ .[]
          | select(((.name // .context // "") | test($adv)) | not) ]
        | (map(select((.conclusion // .state) == null
              or ((.conclusion // .state) | ascii_upcase
                  | . == "PENDING" or . == "EXPECTED"
                    or . == "QUEUED" or . == "IN_PROGRESS"))) | length) as $pending
        | (map(select((.conclusion // .state) != null
              and ((.conclusion // .state) | ascii_upcase
                   | . == "FAILURE" or . == "CANCELLED" or . == "TIMED_OUT"
                     or . == "ACTION_REQUIRED" or . == "ERROR"
                     or . == "STARTUP_FAILURE"))) | length) as $bad
        | "\($pending) \($bad) \(length)"'
}

if [ "$REQUIRE_CHECKS" = "1" ]; then
    _deadline=$(( $(date +%s) + CHECKS_TIMEOUT ))
    while :; do
        if ! _rollup="$(gh pr view "$PR" --repo "$REPO" --json statusCheckRollup \
                --jq '.statusCheckRollup // []' 2>/dev/null)"; then
            _cleanup
            _die "could not read the check rollup for PR #$PR (fail-closed, not merged)"
        fi
        if ! _counts="$(_rollup_counts "$_rollup")"; then
            _cleanup
            _die "could not parse the check rollup for PR #$PR (fail-closed, not merged)"
        fi
        _pending="${_counts%% *}"
        _rest="${_counts#* }"
        _bad="${_rest%% *}"
        _total="${_rest##* }"

        if [ "$_bad" != "0" ]; then
            gh pr comment "$PR" --repo "$REPO" --body "$(printf 'Refused by the merge-time CI gate: %s non-advisory check(s) are not green. Not merged.\n\nFix the failing check, or re-run the merge with `--no-require-checks` if the failure is known-unrelated and accepted.' "$_bad")" >/dev/null 2>&1 || true
            printf 'guarded-merge: PR #%s has %s non-advisory check(s) not green\n' "$PR" "$_bad"
            printf 'blocked checks_not_green\n'
            _cleanup
            exit 1
        fi
        if [ "$_total" != "0" ] && [ "$_pending" = "0" ]; then
            printf 'guarded-merge: PR #%s checks green (%s non-advisory)\n' "$PR" "$_total"
            break
        fi
        # An empty rollup is not proof of green: checks may not have registered
        # yet on a freshly pushed head.
        if [ "$(date +%s)" -ge "$_deadline" ]; then
            printf 'guarded-merge: PR #%s still has %s pending / %s total check(s) after %ss\n' \
                "$PR" "$_pending" "$_total" "$CHECKS_TIMEOUT"
            printf 'blocked checks_not_green\n'
            _cleanup
            exit 1
        fi
        sleep "$CHECKS_POLL"
    done
fi

# 5. Allowed (or overridden) and green: perform the admin merge.
_cleanup
# shellcheck disable=SC2086
gh pr merge "$PR" --repo "$REPO" $MERGE_ARGS
printf 'merged fenced_surface_ok\n'
