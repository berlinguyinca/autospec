#!/usr/bin/env bash
# first-failing-commit.sh — automated first-failing-commit report (issue #4108).
#
# When a per-commit check (default: the `main-builds` CI job, see
# .github/workflows/rust.yml) starts failing on `main`, this script reports
# which commit broke it: the newest commit whose run of the check succeeded
# (LAST_GOOD) and the oldest commit after that whose run failed
# (FIRST_FAILING) — the commit an operator should start bisecting from.
#
# The report is the product: exit 0 means "a report was produced" for BOTH
# STATE:ok and STATE:broken. Exit 2 means the report could not be produced
# (bad usage, API failure, or the check has never run) and the caller should
# treat the state as unknown, not as green.
#
# One GitHub API call (the completed `main` runs list carries each run's
# jobs inline) plus at most two commit-subject lookups. No checkout, no
# local git history, no network beyond `gh api`.
#
# Output (stdout, one KEY:VALUE per line):
#   CHECK:<check name>
#   BRANCH:main
#   COMMITS_SCANNED:<n>
#   STATE:ok | broken | unknown
#   LAST_GOOD:<sha | none | unknown>
#   LAST_GOOD_SUBJECT:<first commit line | ->
#   FIRST_FAILING:<sha | none | unknown>
#   FIRST_FAILING_SUBJECT:<first commit line | ->
#   CANCELLED_AFTER_LAST_GOOD:<n | unknown>
#
# `cancelled` is counted separately, never folded into the failure set
# (issue #4199): a cancelled run is absent evidence — it neither proves its
# commit broke the check (no false FIRST_FAILING bisect target) nor that the
# check passed. A window with no failure but with runs cancelled after the
# last good run is therefore `unknown` (exit 2), not `ok`.
#
# Usage:
#   first-failing-commit.sh --repo OWNER/REPO [--check NAME] [--max-commits N]
#
# Engineering rules (AGENTS.md): set -euo pipefail; if/then/fi (no one-sided
# && short-circuits).

set -euo pipefail

REPO=""
CHECK="main-builds"
MAX_COMMITS=30

_die() {
    printf 'first-failing-commit: %s\n' "$1" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:-}"; shift 2 ;;
        --check) CHECK="${2:-}"; shift 2 ;;
        --max-commits) MAX_COMMITS="${2:-}"; shift 2 ;;
        -h|--help) sed -n 's/^# \?//p' "$0" | head -45; exit 0 ;;
        *) _die "unknown option: $1" ;;
    esac
done

[ -n "$REPO" ] || _die "--repo OWNER/REPO is required"
[ -n "$CHECK" ] || _die "--check must not be empty"
printf '%s' "$MAX_COMMITS" | grep -qE '^[1-9][0-9]*$' || _die "--max-commits must be a positive integer (got: $MAX_COMMITS)"

_emit() {
    # _emit <state> <last_good> <first_failing> <n_scanned> <n_cancelled>
    _state="$1"
    _lg="$2"
    _ff="$3"
    _n="$4"
    _nc="$5"
    _lg_subj="-"
    _ff_subj="-"
    if [ "$_state" != "unknown" ]; then
        if [ "$_lg" != "none" ] && [ -n "$_lg" ]; then
            if _s="$(gh api "repos/$REPO/commits/$_lg" --jq '.commit.message' 2>/dev/null | head -n 1)"; then
                if [ -n "$_s" ]; then
                    _lg_subj="$_s"
                fi
            fi
        fi
        if [ "$_ff" != "none" ] && [ -n "$_ff" ]; then
            if _s="$(gh api "repos/$REPO/commits/$_ff" --jq '.commit.message' 2>/dev/null | head -n 1)"; then
                if [ -n "$_s" ]; then
                    _ff_subj="$_s"
                fi
            fi
        fi
    fi
    printf 'CHECK:%s\n' "$CHECK"
    printf 'BRANCH:main\n'
    printf 'COMMITS_SCANNED:%s\n' "$_n"
    printf 'STATE:%s\n' "$_state"
    printf 'LAST_GOOD:%s\n' "$_lg"
    printf 'LAST_GOOD_SUBJECT:%s\n' "$_lg_subj"
    printf 'FIRST_FAILING:%s\n' "$_ff"
    printf 'FIRST_FAILING_SUBJECT:%s\n' "$_ff_subj"
    printf 'CANCELLED_AFTER_LAST_GOOD:%s\n' "$_nc"
}

if ! _runs="$(gh api "repos/$REPO/actions/runs?branch=main&per_page=$MAX_COMMITS&status=completed" 2>/dev/null)"; then
    printf 'first-failing-commit: could not read the completed main runs for %s\n' "$REPO" >&2
    _emit unknown unknown unknown 0 unknown
    exit 2
fi

if ! printf '%s' "$_runs" | jq -e 'type == "object"' >/dev/null 2>&1; then
    printf 'first-failing-commit: unexpected response shape from the runs API for %s\n' "$REPO" >&2
    _emit unknown unknown unknown 0 unknown
    exit 2
fi

# Reduce the runs list to a time-ordered (oldest -> newest) sequence of one
# record per commit: the newest completed run of $CHECK per sha, with that
# run's job conclusion. group_by(max_by) absorbs re-runs of the same commit.
if ! _seq="$(printf '%s' "$_runs" | jq -c --arg check "$CHECK" '
    [ .workflow_runs[]?
      | select((.jobs // []) | map(.name) | index($check) != null) ]
    | group_by(.head_sha)
    | map(max_by(.created_at))
    | map(. as $r
          | ($r.jobs | map(select(.name == $check)) | max_by(.created_at)) as $j
          | { sha: $r.head_sha, created: $r.created_at, conclusion: $j.conclusion })
    | sort_by(.created)')"; then
    printf 'first-failing-commit: failed to parse the runs payload for %s\n' "$REPO" >&2
    _emit unknown unknown unknown 0 unknown
    exit 2
fi

_n="$(printf '%s' "$_seq" | jq 'length')"
if [ "$_n" = "0" ]; then
    _emit unknown none none 0 unknown
    exit 2
fi

# Two-pass: LAST_GOOD is the newest success in the window; FIRST_FAILING is
# the oldest failure strictly after it. With no success in the window, the
# whole window is "after" the (empty) baseline, so the oldest failure of any
# conclusion in the failure set wins.
_lg=""
if ! _lg="$(printf '%s' "$_seq" | jq -r '[ .[] | select(.conclusion == "success") ] | last | .sha // "none"')"; then
    printf 'first-failing-commit: failed to parse the reduced sequence for %s\n' "$REPO" >&2
    _emit unknown unknown unknown "$_n" unknown
    exit 2
fi

_lg_created=""
if ! _lg_created="$(printf '%s' "$_seq" | jq -r '[ .[] | select(.conclusion == "success") ] | last | .created // ""')"; then
    printf 'first-failing-commit: failed to parse the reduced sequence for %s\n' "$REPO" >&2
    _emit unknown unknown unknown "$_n" unknown
    exit 2
fi

if ! _ff="$(printf '%s' "$_seq" | jq -r --arg lg "$_lg_created" '
    [ .[] | select(.conclusion as $c
        | $c == "failure" or $c == "timed_out" or $c == "startup_failure"
          or $c == "action_required")
      | select(.created > $lg) ]
    | first | .sha // "none"')"; then
    printf 'first-failing-commit: failed to parse the reduced sequence for %s\n' "$REPO" >&2
    _emit unknown unknown unknown "$_n" unknown
    exit 2
fi

# Counted separately from the failure set (issue #4199): a cancelled run
# after the last good run never verified its commit, so the window has no
# failure evidence and no green evidence — that is unknown, not ok.
if ! _nc="$(printf '%s' "$_seq" | jq -r --arg lg "$_lg_created" '
    [ .[] | select(.conclusion == "cancelled") | select(.created > $lg) ]
    | length')"; then
    printf 'first-failing-commit: failed to parse the reduced sequence for %s\n' "$REPO" >&2
    _emit unknown unknown unknown "$_n" unknown
    exit 2
fi

if [ "$_ff" = "none" ] || [ -z "$_ff" ]; then
    if [ "$_nc" -gt 0 ]; then
        _emit unknown "$_lg" none "$_n" "$_nc"
        exit 2
    fi
    _emit ok "$_lg" none "$_n" "$_nc"
    exit 0
fi
_emit broken "$_lg" "$_ff" "$_n" "$_nc"
exit 0
