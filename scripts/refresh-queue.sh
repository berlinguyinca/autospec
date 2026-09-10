#!/usr/bin/env bash
# refresh-queue.sh — regenerate the Phase 4 dispatch queue from the live tracker.
#
# Background (issue #3655): the dispatch queue was a hand-maintained snapshot. It
# drifted from the tracker — resolved issues stayed queued (dispatched as no-ops) and
# newly labelled ones sat unqueued (never dispatched). The queue is now regenerated
# from the live tracker (`open + auto-implement`) on every refresh:
#
#   * Regenerates from the tracker, not a snapshot: the fresh `open + auto-implement`
#     set is the source of truth on every run.
#   * Preserves the order of entries still live: surviving issues keep their position
#     from the previous queue, and newly dispatchable ones are appended in tracker
#     (fetch) order. Dispatch is in order, so reshuffling would starve whatever is
#     near the front.
#   * Prunes resolved issues: an entry that is no longer `open + auto-implement`
#     (closed, or label removed) drops out of the queue.
#   * Reports what it changed so operators see the delta instead of inferring it:
#         excluded 29 issues that already have an open PR
#         queue: 202 -> 134, dropped 86 resolved, added 18 newly labelled
#
# Background (issue #3658): the queue also carried issues whose work was already done
# — on a live fleet 21% of queued issues had an open PR (in review, awaiting merge)
# and the dispatcher re-dispatched them. The filter looked only at the PROBLEM state
# (issue open, label present) and ignored the SOLUTION state (an open PR exists). This
# script applies that filter at refresh time too:
#
#   * Excludes every issue that already has an open PR. An issue is "covered" when
#     any open PR has head branch `fix/issue-N`, or its body references the issue
#     with a closing verb ("Closes #N", "Fixes #N", "Resolves #N" — case-insensitive,
#     no partial-number matches such as #36580).
#   * Filtering, not deletion: the script never mutates GitHub state. If a PR is
#     closed unmerged, its issue simply reappears on the next refresh.
#   * `--check N` is the dispatcher's second line of defense: the dispatcher calls
#     it right before dispatch and refuses issues that already have an open PR
#     (exit 2 — the same refusal channel `autospec claim acquire` uses for
#     in-flight issues).
#
# Usage:
#   refresh-queue.sh [refresh options]
#   refresh-queue.sh [refresh options] --check N
#
# Refresh options:
#   --repo OWNER/REPO   target repo (default: current repo, via `gh api repo`)
#   --out FILE          queue file to write (default: ~/.autospec/queue.json)
#
# Output:
#   refresh: the two report lines above on stdout; the queue file is a JSON array
#            of the surviving issue numbers — previous order kept for entries still
#            live, newly dispatchable issues appended in tracker order.
#   check:   silent on pass; one refusal line on stderr on refuse.
#
# Exit codes:
#   0  refresh OK (queue written) / check pass (no open PR for the issue)
#   1  usage error
#   2  check refused: issue N already has an open PR
#   3  refresh/check failed (missing gh or jq, API error, non-array response) —
#      fail-closed: any previous queue file is left untouched.
#
# Implementation note: this script calls `gh api` endpoints rather than the long
# flags of `gh issue list` / `gh pr list`, so the new file introduces no
# undocumented CLI surface (issue #3658 scopes the diff to this file alone).
#
set -euo pipefail

usage() {
    local end
    end="$(grep -n '^set -euo pipefail' "$0" | head -n 1 | cut -d: -f1)"
    sed -n "2,$((end - 1))p" "$0"
}

die_usage() {
    printf 'refresh-queue.sh: %s\n' "$1" >&2
    usage >&2
    exit 1
}

fail() {
    printf 'refresh-queue.sh: %s\n' "$1" >&2
    exit 3
}

repo=""
out_file="$HOME/.autospec/queue.json"
check_issue=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --repo)
            [ $# -ge 2 ] || die_usage "option $1 needs a value"
            repo="$2"
            shift 2
            ;;
        --out)
            [ $# -ge 2 ] || die_usage "option $1 needs a value"
            out_file="$2"
            shift 2
            ;;
        --check)
            [ $# -ge 2 ] || die_usage "option $1 needs a value"
            check_issue="$2"
            shift 2
            ;;
        *)
            die_usage "unknown argument: $1"
            ;;
    esac
done

for tool in gh jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        fail "required tool not found on PATH: $tool"
    fi
done

resolve_repo() {
    if [ -n "$repo" ]; then
        return 0
    fi
    local out
    if ! out="$(gh api repo 2>&1)"; then
        fail "cannot resolve current repo (run inside a git checkout or pass the repo option): $out"
    fi
    repo="$(jq -r '.full_name // empty' <<<"$out")"
    if [ -z "$repo" ]; then
        fail "gh api repo did not return full_name"
    fi
}

# fetch_pages ENDPOINT PAGES_FILE
# Paginate ENDPOINT (100 items per page), appending each page's JSON array to
# PAGES_FILE (one array per line). Fails closed on any non-array response.
fetch_pages() {
    local endpoint="$1"
    local pages_file="$2"
    local page=1
    local max_pages=100
    : > "$pages_file"
    while [ "$page" -le "$max_pages" ]; do
        local out
        if ! out="$(gh api "${endpoint}&page=${page}" 2>&1)"; then
            fail "gh api call failed (page ${page}): $out"
        fi
        if ! jq -e 'type == "array"' >/dev/null 2>&1 <<<"$out"; then
            fail "non-array response from GitHub API (page ${page}): $(head -c 200 <<<"$out")"
        fi
        printf '%s\n' "$out" >> "$pages_file"
        local count
        count="$(jq 'length' <<<"$out")"
        if [ "$count" -lt 100 ]; then
            return 0
        fi
        page=$((page + 1))
    done
    fail "pagination limit exceeded (100 pages) for ${endpoint}"
}

# run_check PAGES_FILE
# Dispatcher gate: exit 2 with a refusal line when an open PR covers $check_issue;
# exit 0 when it does not.
run_check() {
    local prs_pages="$1"
    local n="$check_issue"
    case "$n" in
        ''|*[!0-9]*)
            die_usage "check issue must be a positive integer"
            ;;
    esac
    local match
    if ! match="$(check_issue="$n" jq -s '
        (env.check_issue | tonumber) as $issue
        | (add // []) as $prs
        | [ $prs[]
            | select(
                (.head.ref == ("fix/issue-" + ($issue | tostring)))
                or (((.body // "") | test(
                    "(?i)\\b(close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)[[:space:]]+#"
                    + ($issue | tostring) + "([^0-9]|$)"))))
          ]
        | .[0] // empty' "$prs_pages")"; then
        fail "jq failed while matching open PRs"
    fi
    if [ -n "$match" ]; then
        local pr_number pr_branch
        pr_number="$(jq -r '.number' <<<"$match")"
        pr_branch="$(jq -r '.head.ref' <<<"$match")"
        printf 'refresh-queue.sh: issue %s already has an open PR: #%s (head %s) — refusing dispatch\n' \
            "$n" "$pr_number" "$pr_branch" >&2
        exit 2
    fi
    exit 0
}

# write_queue FILE
# Atomically replace FILE with $kept_json (tmp + mv in the target directory,
# 0600 file; 0700 directory when it is the default ~/.autospec state dir).
write_queue() {
    local file="$1"
    local dir tmp
    dir="$(dirname -- "$file")"
    if ! mkdir -p -- "$dir" 2>/dev/null; then
        fail "cannot create queue directory: $dir"
    fi
    if [ "$dir" = "$HOME/.autospec" ]; then
        chmod 700 -- "$dir"
    fi
    if ! tmp="$(mktemp -- "$dir/.refresh-queue.XXXXXX")"; then
        fail "cannot create temporary queue file in $dir"
    fi
    if ! jq -c '.' <<<"$kept_json" > "$tmp"; then
        rm -f -- "$tmp"
        fail "cannot serialize queue"
    fi
    chmod 600 -- "$tmp"
    if ! mv -f -- "$tmp" "$file"; then
        rm -f -- "$tmp"
        fail "cannot replace queue file: $file"
    fi
}

resolve_repo

work_dir="$(mktemp -d)"
trap 'rm -rf -- "$work_dir"' EXIT

prs_file="$work_dir/pulls.jsonl"
fetch_pages "repos/${repo}/pulls?state=open&per_page=100" "$prs_file"

if [ -n "$check_issue" ]; then
    run_check "$prs_file"
fi

issues_file="$work_dir/issues.jsonl"
fetch_pages "repos/${repo}/issues?labels=auto-implement&state=open&per_page=100" "$issues_file"

# /issues also returns pull requests; drop anything that is a PR, then keep numbers.
# Keep tracker (fetch) order — NOT sorted — because the queue is dispatched in order
# and reshuffling would starve whatever is near the front (issue #3655).
issues_json="$(jq -s 'add // [] | map(select(.pull_request == null)) | map(.number)' "$issues_file")"
prs_json="$(jq -s 'add // []' "$prs_file")"
printf '%s' "$issues_json" > "$work_dir/issues.json"
printf '%s' "$prs_json" > "$work_dir/prs.json"

# Issues that already have an open PR (the #4033 solution-state filter).
excluded_json="$(jq -n '
    input as $issues
    | input as $prs
    | [ $issues[]
        | . as $num
        | select(any($prs[];
            (.head.ref == ("fix/issue-" + ($num | tostring)))
            or (((.body // "") | test(
                "(?i)\\b(close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)[[:space:]]+#"
                + ($num | tostring) + "([^0-9]|$)")))))
        | $num
      ]' "$work_dir/issues.json" "$work_dir/prs.json")"
printf '%s' "$excluded_json" > "$work_dir/excluded.json"

# Dispatchable = open + auto-implement minus PR-covered, in tracker order.
dispatchable_json="$(jq -n '
    input as $issues
    | input as $excluded
    | $issues - $excluded' "$work_dir/issues.json" "$work_dir/excluded.json")"
printf '%s' "$dispatchable_json" > "$work_dir/dispatchable.json"

# Previous queue, read from the same file we are about to rewrite. A missing, empty,
# or malformed file counts as a first run (empty previous queue).
old_queue_json="$(jq -c 'if type == "array" then map(tonumber) else [] end' "$out_file" 2>/dev/null || printf '[]')"
printf '%s' "$old_queue_json" > "$work_dir/old.json"

# Regenerate: surviving entries keep their previous position; newly dispatchable
# entries are appended in tracker order. dropped = previous entries no longer live
# (typically resolved); added = dispatchable entries new to the queue.
report_json="$(jq -n '
    input as $o
    | input as $d
    | [ $o[] | select(. as $n | $d | index($n) != null) ] as $surviving
    | [ $d[] | select(. as $n | $o | index($n) == null) ] as $added
    | ($surviving + $added) as $queue
    | { queue: $queue,
        old_count: ($o | length),
        new_count: ($queue | length),
        added: ($added | length),
        dropped: ([ $o[] | select(. as $n | $d | index($n) == null) ] | length) }' "$work_dir/old.json" "$work_dir/dispatchable.json")"

kept_json="$(jq -c '.queue' <<<"$report_json")"
old_count="$(jq -r '.old_count' <<<"$report_json")"
new_count="$(jq -r '.new_count' <<<"$report_json")"
added_count="$(jq -r '.added' <<<"$report_json")"
dropped_count="$(jq -r '.dropped' <<<"$report_json")"
excluded_count="$(jq 'length' <<<"$excluded_json")"

printf 'excluded %s issues that already have an open PR\n' "$excluded_count"
printf 'queue: %s -> %s, dropped %s resolved, added %s newly labelled\n' \
    "$old_count" "$new_count" "$dropped_count" "$added_count"

write_queue "$out_file"
