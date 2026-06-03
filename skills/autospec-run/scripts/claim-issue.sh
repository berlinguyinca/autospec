#!/usr/bin/env bash
# claim-issue.sh — atomically claim an autospec auto-implement issue.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
RUN_STATE="$SCRIPT_DIR/run-state.sh"

# Lock-comment markers — must match run-state.sh so the loser self-clean can
# locate this worker's own marked comment.
BEGIN_MARKER="<!-- autospec-run-state:begin -->"
END_MARKER="<!-- autospec-run-state:end -->"

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

verified_state_json="$("$RUN_STATE" read --issue "$issue" --repo "$repo" 2>/dev/null || true)"
verified_owner="$(printf '%s\n' "$verified_state_json" | jq -r '.worker_id // empty' 2>/dev/null || true)"
verified_state="$(printf '%s\n' "$verified_state_json" | jq -r '.state // empty' 2>/dev/null || true)"
if [ "$verified_owner" != "$worker_id" ] || [ "$verified_state" != "claimed" ]; then
    # Lost race: the lowest-id lock comment is owned by a different worker.
    # Self-clean by deleting ONLY this worker's own marked lock comment (the
    # higher id), never the winner's lower-id comment. Locate our own comment
    # by matching worker_id inside the marked-comment body; fail-closed if it
    # cannot be found or the delete fails.
    own_comment_id="$(gh api "repos/$repo/issues/$issue/comments" --jq '. // []' 2>/dev/null \
        | jq -r --arg begin "$BEGIN_MARKER" --arg end "$END_MARKER" --arg wid "$worker_id" '
            map(select((.body // "") | contains($begin) and contains($end)))
            | map(select((.body // "") | contains("\"worker_id\": \"" + $wid + "\"")
                                       or contains("\"worker_id\":\"" + $wid + "\"")))
            | sort_by(.id)
            | (.[-1].id // empty)
        ' 2>/dev/null || true)"
    if [ -n "$own_comment_id" ] && [ "$own_comment_id" != "null" ]; then
        gh api "repos/$repo/issues/comments/$own_comment_id" -X DELETE >/dev/null 2>&1 || true
    fi
    printf 'claim-issue: claim lost (issue %s owned by %s)\n' "$issue" "$verified_owner" >&2
    jq -n \
        --argjson issue "$issue" \
        --arg repo "$repo" \
        --arg worker_id "$worker_id" \
        --arg owner "$verified_owner" \
        --arg state "$verified_state" \
        '{claimed:false, issue:$issue, repo:$repo, worker_id:$worker_id, reason:"claim_lost", observed_owner:$owner, observed_state:$state}'
    exit 2
fi

jq -n \
    --argjson issue "$issue" \
    --arg repo "$repo" \
    --arg worker_id "$worker_id" \
    --arg branch "$branch" \
    '{claimed:true, issue:$issue, repo:$repo, worker_id:$worker_id, branch:$branch}'
