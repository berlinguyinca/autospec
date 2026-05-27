#!/usr/bin/env bash
# claim-issue.sh — atomically claim an autospec auto-implement issue.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
RUN_STATE="$SCRIPT_DIR/run-state.sh"

usage() {
    cat <<'EOF'
Usage: claim-issue.sh --issue <N> [--repo owner/repo] [--worker-id <id>] [--branch <branch>]

Exit codes:
  0  issue claimed
  2  issue was already claimed or is not auto-implement-ready
EOF
}

die() {
    printf 'claim-issue: %s\n' "$1" >&2
    exit 1
}

issue=""
repo=""
worker_id="${AUTOSPEC_WORKER_ID:-}"
branch=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --issue) issue="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --worker-id) worker_id="${2:-}"; shift 2 ;;
        --branch) branch="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$issue" ] || die "--issue is required"
case "$issue" in *[!0-9]*|'') die "--issue must be an integer" ;; esac

if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

if [ -z "$worker_id" ]; then
    host="$(hostname 2>/dev/null || printf 'unknown-host')"
    user="${USER:-unknown-user}"
    worker_id="${host}:${user}:shell:$$:$(date -u +%s)"
fi

labels="$(gh issue view "$issue" --repo "$repo" --json labels --jq '.labels[].name' 2>/dev/null || true)"
if ! printf '%s\n' "$labels" | grep -Fx auto-implement >/dev/null 2>&1; then
    jq -n --argjson issue "$issue" --arg repo "$repo" --arg reason "not_auto_implement" \
        '{claimed:false, issue:$issue, repo:$repo, reason:$reason}'
    exit 2
fi

gh label create in-progress-by-bot --repo "$repo" --color ededed --force >/dev/null 2>&1 || true
if ! gh issue edit "$issue" --repo "$repo" --remove-label auto-implement --add-label in-progress-by-bot >/dev/null; then
    jq -n --argjson issue "$issue" --arg repo "$repo" --arg reason "label_mutation_failed" \
        '{claimed:false, issue:$issue, repo:$repo, reason:$reason}'
    exit 2
fi

"$RUN_STATE" upsert \
    --issue "$issue" \
    --repo "$repo" \
    --worker-id "$worker_id" \
    --state claimed \
    --step claimed \
    --branch "$branch" >/dev/null

jq -n \
    --argjson issue "$issue" \
    --arg repo "$repo" \
    --arg worker_id "$worker_id" \
    --arg branch "$branch" \
    '{claimed:true, issue:$issue, repo:$repo, worker_id:$worker_id, branch:$branch}'
