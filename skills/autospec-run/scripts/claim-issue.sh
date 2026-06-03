#!/usr/bin/env bash
# claim-issue.sh — atomically claim an autospec auto-implement issue.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
RUN_STATE="$SCRIPT_DIR/run-state.sh"

# Lock-comment markers — must match run-state.sh so the loser self-clean can
# locate this worker's own marked comment.
begin_marker="<!-- autospec-run-state:begin -->"
end_marker="<!-- autospec-run-state:end -->"

# own_marked_comment_id REPO ISSUE WORKER — print the highest id among marked
# lock comments whose body carries this worker's worker_id (its own lock); empty
# if none. Used on the lost-race path to self-delete only this worker's comment.
own_marked_comment_id() {
    gh api "repos/$1/issues/$2/comments" --jq '. // []' 2>/dev/null \
        | jq -r --arg b "$begin_marker" --arg e "$end_marker" --arg wid "$3" \
            'map(select((.body//"")|contains($b) and contains($e)))
             | map(select((.body//"")|test("\"worker_id\"\\s*:\\s*\""+$wid+"\"")))
             | sort_by(.id) | (.[-1].id // empty)' 2>/dev/null || true
}

# lowest_lock_field REPO ISSUE FIELD — print FIELD (.id, .updated_at, or the
# embedded worker_id) of the LOWEST-numeric-id marked lock comment, i.e. the CAS
# linearization point. updated_at is the SERVER timestamp from the GitHub API —
# never a local clock. Empty if no marked lock exists.
lowest_lock_field() {
    gh api "repos/$1/issues/$2/comments" --jq '. // []' 2>/dev/null \
        | jq -r --arg b "$begin_marker" --arg e "$end_marker" --arg f "$3" '
            map(select((.body//"")|contains($b) and contains($e)))
            | sort_by(.id) | .[0] as $c
            | if $c == null then ""
              elif $f == "worker_id" then
                ($c.body | capture("\"worker_id\"\\s*:\\s*\"(?<w>[^\"]*)\"").w // "")
              else ($c[$f] // "") end' 2>/dev/null || true
}

# iso_to_epoch ISO8601Z — parse a server UTC timestamp (YYYY-MM-DDThh:mm:ssZ) to
# epoch seconds, portable across BSD (date -j) and GNU (date -d). Empty on parse
# failure so callers can fail closed (treat as not-stale).
iso_to_epoch() {
    [ -n "$1" ] || return 0
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s 2>/dev/null \
        || date -u -d "$1" +%s 2>/dev/null \
        || true
}

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

# Fixed evaluation order BEFORE the destructive upsert (spec §Critical-improvement
# fold-in): (1) determine the lowest-id marked lock = current owner; (2) if a
# DIFFERENT worker owns it and that lock is FRESH (server updated_at age <= ttl)
# -> claim lost, self-clean, exit 2 — a live-but-slow owner is never reclaimed,
# and the fresh winner's lock is never overwritten; (3) ONLY if the lowest-id
# lock is STALE do we fall through to the upsert/reclaim path below. No existing
# lock, or a lock we already own, also falls through (normal claim/refresh).
reclaim_secs="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
case "$reclaim_secs" in *[!0-9]*|'') reclaim_secs=10800 ;; esac
lowest_owner="$(lowest_lock_field "$repo" "$issue" worker_id)"
if [ -n "$lowest_owner" ] && [ "$lowest_owner" != "$worker_id" ]; then
    lowest_updated_at="$(lowest_lock_field "$repo" "$issue" updated_at)"
    lock_epoch="$(iso_to_epoch "$lowest_updated_at")"
    now_epoch="$(date -u +%s)"
    # Fail closed: an unparseable server timestamp is treated as FRESH (age 0),
    # never stale, so we never reclaim on ambiguity.
    age=0
    if [ -n "$lock_epoch" ]; then age=$(( now_epoch - lock_epoch )); fi
    if [ "$age" -le "$reclaim_secs" ]; then
        own_comment_id="$(own_marked_comment_id "$repo" "$issue" "$worker_id")"
        if [ -n "$own_comment_id" ] && [ "$own_comment_id" != "null" ]; then
            gh api "repos/$repo/issues/comments/$own_comment_id" -X DELETE >/dev/null 2>&1 || true
        fi
        printf 'claim-issue: claim lost (issue %s owned by %s, lock fresh)\n' "$issue" "$lowest_owner" >&2
        jq -n \
            --argjson issue "$issue" \
            --arg repo "$repo" \
            --arg worker_id "$worker_id" \
            --arg owner "$lowest_owner" \
            '{claimed:false, issue:$issue, repo:$repo, worker_id:$worker_id, reason:"claim_lost", observed_owner:$owner}'
        exit 2
    fi
    printf 'claim-issue: stale lease reclaimed (issue %s, prior owner %s, age %ss > ttl %ss)\n' \
        "$issue" "$lowest_owner" "$age" "$reclaim_secs" >&2
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
    # Self-clean by deleting ONLY this worker's own marked lock comment, never
    # the winner's lower-id comment. Fail-closed if it cannot be found.
    own_comment_id="$(own_marked_comment_id "$repo" "$issue" "$worker_id")"
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
