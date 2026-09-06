#!/usr/bin/env sh
# scripts/verify-produced-work.sh — decide whether a task produced work, counting
# BOTH uncommitted changes and commits ahead of the base.
#
# Why this exists: the produced-work check on this repo asked "is
# `git status --porcelain` empty?" and treated an empty answer as "the task did
# nothing" — `worktree-guard.sh` asserts dirtiness that way, and
# `autospec-worker-v1.sh` computes `git_changed_files` from
# `git status --porcelain` alone. A subagent that committed its work and left a
# clean tree is therefore indistinguishable from a subagent that did nothing at
# all, and the gate failed the one that actually finished. Symmetrically, a
# `git rev-list` that errored returned nothing, and nothing was read as zero
# commits ahead.
#
# The rule this script implements:
#
#   produced = uncommitted_changes > 0 OR commits_ahead > 0
#
# and each side of that OR is measured independently. A measurement that fails
# is recorded as `unknown`, and `unknown` is NOT 0: if either side could not be
# measured the verdict is `unknown` (exit 2), never `no work produced` (exit 1).
# Concluding "nothing happened" from "I could not look" is the bug.
#
# Exit codes (same vocabulary as ui-evidence-gates.sh and verify-gate.sh):
#
#   0  WORK         work produced (dirty tree, or commits ahead, or both)
#   1  NONE         both sides measured, both are zero — nothing was produced
#   2  UNKNOWN      a measurement could not be taken; no verdict is available
#   3  UNAVAILABLE  a required tool is not on PATH; nothing was measured
#   64               usage error

set -eu

usage() {
    cat <<'EOF'
Usage:
  verify-produced-work.sh [--repo-root <dir>] [--base <ref>] [--json]

  --repo-root <dir>  repository to inspect (default: current directory)
  --base <ref>       base revision for the commits-ahead count. Defaults to
                     $AUTOSPEC_BASE_REF, then the branch's own upstream. With
                     none of the three resolvable the commits-ahead side is
                     `unknown`, so the verdict is unknown rather than "no work".
  --json             emit the status record only, no human line

Status record:

  produced-work: yes|no|unknown (uncommitted_changes=<int>|"unknown",
                                 commits_ahead=<int>|"unknown", base=<ref>|"unknown")

Exit codes: 0 work produced, 1 measured and none, 2 unknown, 3 tool unavailable,
64 usage error.
EOF
}

die_usage() {
    printf 'verify-produced-work: %s\n' "$*" >&2
    usage >&2
    exit 64
}

REPO_ROOT="$(pwd)"
BASE="${AUTOSPEC_BASE_REF:-}"
JSON_ONLY=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root)
            [ "$#" -ge 2 ] || die_usage "--repo-root requires a value"
            REPO_ROOT="$2"
            shift 2
            ;;
        --base)
            [ "$#" -ge 2 ] || die_usage "--base requires a value"
            BASE="$2"
            shift 2
            ;;
        --json)
            JSON_ONLY=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die_usage "unknown argument: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die_usage "--repo-root does not exist: $REPO_ROOT"

# json_string_array <words...> — a JSON array of strings, built without sed so
# that the tool-assertion path needs no tool that might itself be missing.
json_string_array() {
    printf '['
    first=1
    for item in "$@"; do
        [ "$first" -eq 1 ] || printf ','
        first=0
        printf '"%s"' "$item"
    done
    printf ']'
}

# ---------------------------------------------------------------------------
# Rule 1 — assert the toolchain before measuring.
#
# `git` is the only tool this gate reads from, and `awk` is the counter. Check
# both before the first measurement: with git absent every command below prints
# nothing to stdout, and an empty stdout read as a count is exactly "0 changes,
# 0 commits ahead" — the verdict `no work produced`, for a repository that was
# never looked at.
# ---------------------------------------------------------------------------
MISSING_COUNT=0
MISSING_LIST=""
for tool in git awk; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        MISSING_COUNT=$((MISSING_COUNT + 1))
        MISSING_LIST="$MISSING_LIST $tool"
    fi
done

if [ "$MISSING_COUNT" -gt 0 ]; then
    # shellcheck disable=SC2086  # word splitting of the list is the point
    for tool in $MISSING_LIST; do
        printf 'verify-produced-work: UNAVAILABLE — missing tool: %s\n' "$tool" >&2
    done
    if [ "$JSON_ONLY" -eq 1 ]; then
        # shellcheck disable=SC2086  # word splitting of the list is the point
        printf '{"schema":1,"status":"UNAVAILABLE","rule":"tool-unavailable","missing_tools":%s,"uncommitted_changes":"unknown","commits_ahead":"unknown","base":"unknown"}\n' \
            "$(json_string_array $MISSING_LIST)"
    fi
    exit 3
fi

cd "$REPO_ROOT"

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    printf 'verify-produced-work: UNKNOWN — not a git repository: %s\n' "$REPO_ROOT" >&2
    if [ "$JSON_ONLY" -eq 1 ]; then
        printf '{"schema":1,"status":"UNKNOWN","rule":"not-a-repository","uncommitted_changes":"unknown","commits_ahead":"unknown","base":"unknown"}\n'
    fi
    exit 2
fi

# json_number_or_unknown <value> — a measured count is a JSON number; the token
# `unknown` is the JSON string "unknown". Never 0: collapsing the two is the
# defect this gate was written against.
json_number_or_unknown() {
    case "$1" in
        '' | unknown) printf '"unknown"' ;;
        *[!0-9]*) printf '"unknown"' ;;
        *) printf '%s' "$1" ;;
    esac
}

# count_nonempty <file> — count of lines the producer actually printed.
#
# The producer's exit status is checked by the caller and its output is captured
# without a pipe, so a git error cannot reach here at all: an empty file means
# "zero entries", which is what it claims to mean.
count_nonempty() {
    printf '%s\n' "$1" | awk 'NF > 0 { n++ } END { printf "%d\n", n + 0 }'
}

# as_count <value> — pass a non-negative integer through, everything else
# becomes the token `unknown`. Guards against a git that prints a warning, a
# usage string, or nothing at all on stdout.
as_count() {
    case "$1" in
        '' | *[!0-9]*) printf 'unknown' ;;
        *) printf '%s' "$1" ;;
    esac
}

# --- measurement 1: uncommitted changes -------------------------------------
uncommitted='unknown'
if dirt_out="$(git status --porcelain=v1 --untracked-files=all 2>/dev/null)"; then
    uncommitted="$(as_count "$(count_nonempty "$dirt_out")")"
fi

# --- measurement 2: commits ahead of base -----------------------------------
base='unknown'
commits_ahead='unknown'

if [ -z "$BASE" ]; then
    BASE="$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || true)"
fi

if [ -n "$BASE" ]; then
    if git rev-parse --verify --quiet "$BASE^{commit}" >/dev/null 2>&1; then
        base="$BASE"
        if ahead_out="$(git rev-list --count "$BASE..HEAD" 2>/dev/null)"; then
            # Command substitution strips the trailing newline, so the raw
            # stdout of `rev-list --count` is already a bare integer.
            commits_ahead="$(as_count "$ahead_out")"
        else
            printf 'verify-produced-work: UNKNOWN — `git rev-list --count %s..HEAD` failed\n' "$BASE" >&2
        fi
    fi
fi

# --- verdict -----------------------------------------------------------------
# Monotone in what was measured. `yes` needs only one positive side, so an
# unknown commits-ahead count cannot erase a dirty tree. `no` needs BOTH sides
# measured at zero — that is the only shape allowed to say nothing was produced.
# Everything else is `unknown`, which is exit 2 and is not a failure of the
# task under test.
if { [ "$uncommitted" != 'unknown' ] && [ "$uncommitted" -gt 0 ]; } ||
    { [ "$commits_ahead" != 'unknown' ] && [ "$commits_ahead" -gt 0 ]; }; then
    verdict='yes'
    verdict_upper='WORK'
    exit_code=0
    rule='work-produced'
elif [ "$uncommitted" != 'unknown' ] && [ "$commits_ahead" != 'unknown' ]; then
    verdict='no'
    verdict_upper='NONE'
    exit_code=1
    rule='no-work-produced'
else
    verdict='unknown'
    verdict_upper='UNKNOWN'
    exit_code=2
    rule='measurement-unavailable'
fi

if [ "$JSON_ONLY" -eq 1 ]; then
    printf '{"schema":1,"status":"%s","rule":"%s","uncommitted_changes":%s,"commits_ahead":%s,"base":"%s"}\n' \
        "$verdict_upper" "$rule" \
        "$(json_number_or_unknown "$uncommitted")" \
        "$(json_number_or_unknown "$commits_ahead")" "$base"
    exit "$exit_code"
fi

printf 'produced-work: %s (uncommitted_changes=%s, commits_ahead=%s, base=%s)\n' \
    "$verdict" "$uncommitted" "$commits_ahead" "$base"
exit "$exit_code"
