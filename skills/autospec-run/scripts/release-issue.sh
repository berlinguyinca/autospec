#!/usr/bin/env bash
# release-issue.sh — release or fail an autospec distributed issue claim.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
RUN_STATE="$SCRIPT_DIR/run-state.sh"
TERMINAL_BEGIN_MARKER="<!-- autospec-run-terminal:begin -->"
TERMINAL_END_MARKER="<!-- autospec-run-terminal:end -->"

usage() {
    cat <<'EOF'
Usage: release-issue.sh --issue <N> [--repo owner/repo] [--worker-id <id>] [--state released|failed|merged] [--branch <branch>] [--pr <PR>]
EOF
}

die() {
    printf 'release-issue: %s\n' "$1" >&2
    exit 1
}

terminal_merged_exists() {
    gh api "repos/$1/issues/$2/comments" --jq '. // []' 2>/dev/null \
        | jq -e --arg b "$TERMINAL_BEGIN_MARKER" --arg e "$TERMINAL_END_MARKER" '
            any(.[]; ((.body//"")|contains($b) and contains($e)) and
              (((.body//"")|capture("\"state\"\\s*:\\s*\"(?<s>[^\"]*)\"").s // "") == "merged"))
          ' >/dev/null 2>&1
}

create_terminal_merged_comment() {
    repo="$1"
    issue="$2"
    worker_id="$3"
    branch="$4"
    pr="$5"
    if terminal_merged_exists "$repo" "$issue"; then
        return 0
    fi
    now_iso="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    state_json="$(jq -n \
        --arg repo "$repo" \
        --arg issue "$issue" \
        --arg worker_id "$worker_id" \
        --arg branch "$branch" \
        --arg pr "$pr" \
        --arg finalized_at "$now_iso" \
        '{schema:1, repo:$repo, issue:($issue|tonumber), worker_id:$worker_id, state:"merged", branch:$branch, pr:$pr, finalized_at:$finalized_at}')"
    body_file="$(mktemp -t autospec-run-terminal.XXXXXX)"
    trap 'rm -f "$body_file"' EXIT
    {
        printf '%s\n' "$TERMINAL_BEGIN_MARKER"
        printf '%s\n' "$state_json"
        printf '%s\n' "$TERMINAL_END_MARKER"
    } > "$body_file"
    gh issue comment "$issue" --repo "$repo" --body-file "$body_file" >/dev/null
}

issue=""
repo=""
worker_id="${AUTOSPEC_WORKER_ID:-}"
state="released"
branch=""
pr=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --issue) issue="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --worker-id) worker_id="${2:-}"; shift 2 ;;
        --state) state="${2:-}"; shift 2 ;;
        --branch) branch="${2:-}"; shift 2 ;;
        --pr) pr="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$issue" ] || die "--issue is required"
case "$issue" in *[!0-9]*|'') die "--issue must be an integer" ;; esac
case "$state" in released|failed|merged) ;; *) die "--state must be released, failed, or merged" ;; esac

if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

if [ -z "$worker_id" ]; then
    host="$(hostname 2>/dev/null || printf 'unknown-host')"
    user="${USER:-unknown-user}"
    worker_id="${host}:${user}:shell:$$:$(date -u +%s)"
fi

if [ "$state" = "merged" ]; then
    create_terminal_merged_comment "$repo" "$issue" "$worker_id" "$branch" "$pr"
fi

"$RUN_STATE" upsert \
    --issue "$issue" \
    --repo "$repo" \
    --worker-id "$worker_id" \
    --state "$state" \
    --step "$state" \
    --branch "$branch" \
    --pr "$pr" >/dev/null

case "$state" in
    merged)
        gh issue edit "$issue" --repo "$repo" --remove-label in-progress-by-bot >/dev/null 2>&1 || true
        ;;
    *)
        gh issue edit "$issue" --repo "$repo" --remove-label in-progress-by-bot --add-label auto-implement >/dev/null 2>&1 || true
        ;;
esac

jq -n \
    --argjson issue "$issue" \
    --arg repo "$repo" \
    --arg worker_id "$worker_id" \
    --arg state "$state" \
    '{released:true, issue:$issue, repo:$repo, worker_id:$worker_id, state:$state}'
