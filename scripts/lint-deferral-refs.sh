#!/usr/bin/env bash
# scripts/lint-deferral-refs.sh — a promise that deferred work survives on a ref
# must name a ref that exists.
#
# Rule: DEFERRAL_REF_ABSENT.
#
# Why this exists (#3497). #3379 deferred ~383 lines of Rust with:
#
#   "The remaining Rust stays on `feat/pi-agent-handoffs`, with
#    `backup/pi-agent-handoffs-prerebase` and tag `pi-handoffs-prerebase-20260825`
#    as safety refs."
#
# All three were later deleted. Nothing objected, because the promise lived in
# prose no gate reads, and the PR still reads as safely parked. The work is
# unrecoverable — absent from every remote branch, every pull ref, every clone on
# the host, and the dangling-object store. A merged PR is a record; when it says
# work survives somewhere, that has to stay true or stop being said.
#
# What is and is not a finding. Mentioning a branch is ordinary — branches are
# merged and deleted constantly, and failing on every mention would make this
# noise and get it switched off. The finding is a *promise*: a sentence claiming
# deferred work is preserved on, parked on, stays on, or is kept as a safety ref.
# Only refs named inside such a sentence are checked, and only those carrying a
# '/' or '-'. A bare token like `v1.2.3` or `main` is not treated as a ref: it is
# shapeless enough that checking it would flag ordinary prose. Every branch and
# dated tag in this repository carries one or the other.
#
# Usage:
#   lint-deferral-refs.sh --body-file <path> [--remote <url|path>]
#   lint-deferral-refs.sh --pr <N> [--repo <owner/name>] [--remote <url|path>]
#   lint-deferral-refs.sh --help
#
# Output: one finding per stdout line
#   DEFERRAL_REF_ABSENT:<ref>: <description>
#
# Exit codes:
#   0  no promise, or every promised ref exists
#   1  usage error (unreadable body, missing gh, bad arguments)
#   N  number of absent promised refs (capped at 64)

set -uo pipefail

REMOTE="${AUTOSPEC_DEFERRAL_REMOTE:-origin}"
BODY_FILE=""
PR_NUMBER=""
REPO="${AUTOSPEC_DEFERRAL_REPO:-}"

usage() {
    cat <<'EOF'
Usage: lint-deferral-refs.sh --body-file <path> [--remote <url|path>]
       lint-deferral-refs.sh --pr <N> [--repo <owner/name>] [--remote <url|path>]

Rule DEFERRAL_REF_ABSENT: a body that promises deferred work survives on a git
ref — "stays on", "remains on", "preserved on", "parked on", "kept on", or names
something "as a safety ref"/"as safety refs"/"as a backup ref" — must name a ref
that still exists on the remote.

Mentioning a branch is not a promise, and is never a finding. Only refs written
in backticks inside a promise sentence are checked.

Exit code is the number of absent promised refs; 0 means pass.
EOF
}

_die() { printf 'lint-deferral-refs: %s\n' "$1" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --body-file) BODY_FILE="${2:-}"; shift 2 ;;
        --pr)        PR_NUMBER="${2:-}"; shift 2 ;;
        --repo)      REPO="${2:-}"; shift 2 ;;
        --remote)    REMOTE="${2:-}"; shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *)           printf 'lint-deferral-refs: unknown argument: %s\n' "$1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -n "$BODY_FILE" ] && [ -n "$PR_NUMBER" ]; then
    _die "pass --body-file or --pr, not both"
fi

BODY="$(mktemp)" || _die "cannot create a temporary file"
trap 'rm -f "$BODY"' EXIT

if [ -n "$BODY_FILE" ]; then
    [ -r "$BODY_FILE" ] || _die "--body-file must point to a readable file: $BODY_FILE"
    cat "$BODY_FILE" > "$BODY"
elif [ -n "$PR_NUMBER" ]; then
    command -v gh >/dev/null 2>&1 || _die "--pr needs the gh CLI on PATH"
    if [ -n "$REPO" ]; then
        gh pr view "$PR_NUMBER" -R "$REPO" --json body --jq '.body' > "$BODY" 2>/dev/null \
            || _die "cannot read the body of PR $PR_NUMBER in $REPO"
    else
        gh pr view "$PR_NUMBER" --json body --jq '.body' > "$BODY" 2>/dev/null \
            || _die "cannot read the body of PR $PR_NUMBER"
    fi
else
    usage >&2
    exit 1
fi

# ── find the promise sentences ────────────────────────────────────────────────
# Sentence-per-line first: a promise routinely wraps across source lines, and a
# line-oriented scan would split "stays on `x`, with `y` ... as safety refs"
# into fragments that individually look innocent.
#
# Two boundaries must survive that join, or unrelated text fuses into one
# enormous "sentence" and any token in it pairs with any phrase in it:
#   * table rows — they carry no terminating punctuation at all. #3481 collapsed
#     a whole capability table into one sentence, pairing a promise phrase in one
#     cell with the repository name `autospec-inferweave` several rows away.
#   * blank lines — a paragraph break ends a thought as surely as a full stop.
SENTENCES="$(
    sed -e '/^[[:space:]]*|/d' -e 's/^[[:space:]]*$/./' "$BODY" \
        | tr '\n' ' ' \
        | sed 's/\([.!?]\)[[:space:]]\{1,\}/\1\n/g'
)"

PROMISE_RE='(stays|stay|remains|remain|lives|live|kept|keep|preserved|parked|retained)[[:space:]]+(on|at|in)|as[[:space:]]+(a[[:space:]]+)?(safety|backup)[[:space:]]+refs?|safety[[:space:]]+refs?'

# ── collect the refs named inside them ────────────────────────────────────────
# A ref is a backticked token that looks like a branch or tag: it must contain a
# '/' or a '-', so plain prose in backticks (`main`, `null`) is not mistaken for
# one. `origin/...` is stripped to its ref name before lookup.
#
# Paths are then excluded, because prose names them constantly and they satisfy
# the same shape. Scanning the twenty most recently merged PRs, the only false
# positives were `backend/agent_lsp.rs` (#3483) and `.autospec/` (#3481). Three
# rules separate them from refs, and none costs a real ref:
#   * a trailing '/' — a ref never ends with one (git rejects it)
#   * a leading '.' or '/' — likewise rejected by git-check-ref-format
#   * a trailing dot followed by an ALPHABETIC extension — `agent_lsp.rs`,
#     `README.md`. A trailing numeric segment stays a ref, so `v1.2.3` survives.
candidates="$(
    printf '%s\n' "$SENTENCES" \
        | grep -iEa "$PROMISE_RE" \
        | grep -oE '`[A-Za-z0-9._/-]+`' \
        | tr -d '`' \
        | grep -E '[/-]' \
        | grep -vE '/$' \
        | grep -vE '^[./]' \
        | grep -vE '\.[A-Za-z][A-Za-z0-9]{0,5}$' \
        | sed 's|^origin/||' \
        | sort -u
)"

[ -n "$candidates" ] || exit 0

# ── verify each against the remote ────────────────────────────────────────────
findings=0
while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    # Ask once for both namespaces; --exit-code is 2 when nothing matches.
    if git ls-remote --heads --tags --exit-code "$REMOTE" \
        "refs/heads/$ref" "refs/tags/$ref" >/dev/null 2>&1; then
        continue
    fi
    printf 'DEFERRAL_REF_ABSENT:%s: named as the resting place of deferred work, but %s has no such branch or tag; either push the ref or delete the claim\n' \
        "$ref" "$REMOTE"
    findings=$((findings + 1))
    [ "$findings" -ge 64 ] && break
done <<EOF
$candidates
EOF

exit "$findings"
