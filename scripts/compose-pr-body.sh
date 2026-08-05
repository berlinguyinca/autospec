#!/usr/bin/env bash
# scripts/compose-pr-body.sh — assemble a PR body from the branch, not from a model.
#
# The Phase 4 implementer used to write the whole PR body inside its subagent
# turn. Everything in it except the summary paragraph is already derivable from
# the branch: the closing reference is the issue number, the change list is the
# commit subjects, and the verification line is the acceptance-criteria suite. A
# model re-deriving those spends output tokens to restate facts git already holds,
# and does it slightly differently every time.
#
# What is deliberately NOT templated: the summary. This repo's history carries
# load-bearing *why* — which alternative was rejected, which failure mode a guard
# prevents — and no diff-derived template can produce that. Pass it through
# --summary-file and it is emitted verbatim. Absent, the body is still valid, just
# thinner; that is a better failure than a confident summary nobody wrote.
#
# Commit messages are likewise untouched. They are written inside the same
# subagent turn and carry the same irreplaceable rationale.
#
# Usage:
#   compose-pr-body.sh --issue <n> [--base <ref>] [--head <ref>]
#                      [--summary-file <path>] [--ac-test <path>]
#
# Defaults: --base origin/${AUTOSPEC_BASE_BRANCH:-main}, --head HEAD,
#           --ac-test tests/ac/issue-<n>.bats
#
# Exit codes:
#   0  a body was printed
#   1  usage error (missing/invalid --issue, unreadable --summary-file)
#   3  no commits in <base>..<head> — there is nothing to open a PR for, and the
#      caller must not run `gh pr create`
#
# bash 3.2+. set -u; if/then/fi one-sided conditionals; no RETURN traps.

set -u

PROG="compose-pr-body"
_die() { printf '%s: %s\n' "$PROG" "$1" >&2; exit "${2:-1}"; }

ISSUE=
BASE="origin/${AUTOSPEC_BASE_BRANCH:-main}"
HEAD_REF="HEAD"
SUMMARY_FILE=
AC_TEST=

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --issue)
            if [ $# -lt 2 ]; then _die '--issue requires a number'; fi
            ISSUE="$2"; shift 2 ;;
        --base)
            if [ $# -lt 2 ]; then _die '--base requires a ref'; fi
            BASE="$2"; shift 2 ;;
        --head)
            if [ $# -lt 2 ]; then _die '--head requires a ref'; fi
            HEAD_REF="$2"; shift 2 ;;
        --summary-file)
            if [ $# -lt 2 ]; then _die '--summary-file requires a path'; fi
            SUMMARY_FILE="$2"; shift 2 ;;
        --ac-test)
            if [ $# -lt 2 ]; then _die '--ac-test requires a path'; fi
            AC_TEST="$2"; shift 2 ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$ISSUE" ]; then _die '--issue is required'; fi
case "$ISSUE" in
    ''|*[!0-9]*) _die "--issue must be a positive integer: $ISSUE" ;;
esac
if [ -n "$SUMMARY_FILE" ] && [ ! -f "$SUMMARY_FILE" ]; then
    _die "--summary-file not found: $SUMMARY_FILE"
fi
if [ -z "$AC_TEST" ]; then AC_TEST="tests/ac/issue-${ISSUE}.bats"; fi

# ── the change list ───────────────────────────────────────────────────────────
# Oldest-first so the body reads in the order the work was done. A commit range
# that resolves to nothing is exit 3, not an empty section: opening a PR with no
# commits is a mistake worth stopping, and a body that merely looks sparse would
# let it through.
_subjects="$(git log --reverse --format='%s' "${BASE}..${HEAD_REF}" 2>/dev/null || printf '')"
if [ -z "$_subjects" ]; then
    _die "no commits in ${BASE}..${HEAD_REF}; nothing to open a PR for" 3
fi

# ── verification line ─────────────────────────────────────────────────────────
# The @test count is read, never run: composing a body must stay side-effect free
# and fast. "Suite absent" is stated rather than omitted, because a silently
# missing verification line reads as "no tests were needed".
_verify=
if [ -f "$AC_TEST" ]; then
    _n="$(grep -c '^[[:space:]]*@test[[:space:]]' "$AC_TEST" 2>/dev/null || printf '0')"
    _verify="$(printf -- '- `bats %s` — %s acceptance test(s)' "$AC_TEST" "$_n")"
else
    _verify="$(printf -- '- no acceptance-criteria suite at `%s`' "$AC_TEST")"
fi

# ── emit ──────────────────────────────────────────────────────────────────────
printf 'Closes #%s\n' "$ISSUE"

if [ -n "$SUMMARY_FILE" ]; then
    printf '\n'
    # Verbatim: the summary is the one part a model contributes, and reflowing or
    # re-wording it here would defeat the point of asking for it.
    cat "$SUMMARY_FILE"
    printf '\n'
fi

printf '\n## Changes\n\n'
printf '%s\n' "$_subjects" | while IFS= read -r _s; do
    printf -- '- %s\n' "$_s"
done

printf '\n## Verification\n\n%s\n' "$_verify"

printf '\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n'
