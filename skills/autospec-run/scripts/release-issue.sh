#!/usr/bin/env bash
# release-issue.sh — release or fail an autospec distributed issue claim.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
RUN_STATE="$SCRIPT_DIR/run-state.sh"

usage() {
    cat <<'EOF'
Usage: release-issue.sh --issue <N> [--repo owner/repo] [--worker-id <id>] [--state released|failed|merged] [--branch <branch>] [--pr <PR>]
EOF
}

die() {
    printf 'release-issue: %s\n' "$1" >&2
    exit 1
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

case "$state" in
    merged)
        gh issue edit "$issue" --repo "$repo" --remove-label in-progress-by-bot >/dev/null 2>&1 || true
        ;;
    *)
        gh issue edit "$issue" --repo "$repo" --remove-label in-progress-by-bot --add-label auto-implement >/dev/null 2>&1 || true
        ;;
esac

"$RUN_STATE" upsert \
    --issue "$issue" \
    --repo "$repo" \
    --worker-id "$worker_id" \
    --state "$state" \
    --step "$state" \
    --branch "$branch" \
    --pr "$pr" >/dev/null

jq -n \
    --argjson issue "$issue" \
    --arg repo "$repo" \
    --arg worker_id "$worker_id" \
    --arg state "$state" \
    '{released:true, issue:$issue, repo:$repo, worker_id:$worker_id, state:$state}'
