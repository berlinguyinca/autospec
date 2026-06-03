#!/usr/bin/env bash
# run-state.sh — read/upsert/clear autospec-run GitHub issue state comments.

set -eu

BEGIN_MARKER="<!-- autospec-run-state:begin -->"
END_MARKER="<!-- autospec-run-state:end -->"

usage() {
    cat <<'EOF'
Usage:
  run-state.sh read   --issue <N> [--repo owner/repo]
  run-state.sh upsert --issue <N> [--repo owner/repo] --worker-id <id> --state <state> [--step <step>] [--branch <b>] [--pr <p>] [--paths <json-or-csv>] [--ttl-seconds <N>]
  run-state.sh clear  --issue <N> [--repo owner/repo]
EOF
}

die() {
    printf 'run-state: %s\n' "$1" >&2
    exit 1
}

command_name="${1:-}"
[ -n "$command_name" ] || { usage; exit 1; }
shift

issue=""
repo=""
worker_id=""
state=""
step=""
branch=""
pr=""
paths="[]"
ttl_seconds="10800"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --issue) issue="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --worker-id) worker_id="${2:-}"; shift 2 ;;
        --state) state="${2:-}"; shift 2 ;;
        --step) step="${2:-}"; shift 2 ;;
        --branch) branch="${2:-}"; shift 2 ;;
        --pr) pr="${2:-}"; shift 2 ;;
        --paths) paths="${2:-}"; shift 2 ;;
        --ttl-seconds) ttl_seconds="${2:-}"; shift 2 ;;
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

comments_json() {
    gh api "repos/$repo/issues/$issue/comments" --jq '. // []'
}

gh_api_retry() {
    attempts=0
    max_attempts="${AUTOSPEC_GH_API_RETRIES:-3}"
    sleep_seconds="${AUTOSPEC_GH_API_RETRY_SLEEP:-1}"
    case "$max_attempts" in *[!0-9]*|'') max_attempts=3 ;; esac
    [ "$max_attempts" -gt 0 ] || max_attempts=1

    while :; do
        if gh api "$@"; then
            return 0
        fi
        attempts=$((attempts + 1))
        if [ "$attempts" -ge "$max_attempts" ]; then
            return 1
        fi
        sleep "$sleep_seconds"
    done
}

state_comment_ids() {
    # Emit marked lock-comment ids sorted numeric-ascending. The lowest id is
    # the CAS linearization point (the single deterministic owner); array/API
    # order is not a contracted monotonic key, so selection never relies on it.
    comments_json | jq -r --arg begin "$BEGIN_MARKER" --arg end "$END_MARKER" '
      map(select((.body // "") | contains($begin) and contains($end))) |
      sort_by(.id) |
      .[].id
    '
}

state_comment_id() {
    # Lowest marked id (first of the ascending-sorted list).
    state_comment_ids | sed -n '1p'
}

state_comment_body() {
    comments_json | jq -r --arg begin "$BEGIN_MARKER" --arg end "$END_MARKER" '
      map(select((.body // "") | contains($begin) and contains($end))) |
      sort_by(.id) |
      if length == 0 then "" else .[0].body end
    '
}

extract_state_json() {
    awk -v begin="$BEGIN_MARKER" -v end="$END_MARKER" '
      $0 == begin { inside=1; next }
      $0 == end { inside=0; exit }
      inside { print }
    '
}

normalize_paths() {
    raw="$1"
    if printf '%s' "$raw" | jq -e 'type == "array"' >/dev/null 2>&1; then
        printf '%s' "$raw"
    elif [ -z "$raw" ]; then
        printf '[]'
    else
        printf '%s' "$raw" | jq -R 'split(",") | map(gsub("^\\s+|\\s+$"; "")) | map(select(length > 0))'
    fi
}

case "$command_name" in
    read)
        body="$(state_comment_body)"
        [ -n "$body" ] || exit 0
        json="$(printf '%s\n' "$body" | extract_state_json)"
        if printf '%s\n' "$json" | jq -e --arg issue "$issue" --arg repo "$repo" \
            '.schema == 1 and (.issue|tostring) == $issue and .repo == $repo' >/dev/null 2>&1; then
            printf '%s\n' "$json" | jq .
        fi
        ;;
    upsert)
        [ -n "$worker_id" ] || die "--worker-id is required for upsert"
        [ -n "$state" ] || die "--state is required for upsert"
        [ -n "$step" ] || step="$state"
        case "$ttl_seconds" in *[!0-9]*|'') die "--ttl-seconds must be an integer" ;; esac

        now_iso="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
        path_json="$(normalize_paths "$paths")"
        existing="$(state_comment_body)"
        claimed_at="$now_iso"
        if [ -n "$existing" ]; then
            existing_json="$(printf '%s\n' "$existing" | extract_state_json)"
            claimed_at="$(printf '%s\n' "$existing_json" | jq -r '.claimed_at // empty' 2>/dev/null || true)"
            [ -n "$claimed_at" ] || claimed_at="$now_iso"
        fi

        state_json="$(jq -n \
            --arg repo "$repo" \
            --arg issue "$issue" \
            --arg worker_id "$worker_id" \
            --arg state "$state" \
            --arg branch "$branch" \
            --arg pr "$pr" \
            --arg step "$step" \
            --argjson paths "$path_json" \
            --arg claimed_at "$claimed_at" \
            --arg updated_at "$now_iso" \
            --argjson ttl_seconds "$ttl_seconds" \
            '{schema:1, repo:$repo, issue:($issue|tonumber), worker_id:$worker_id, state:$state, branch:$branch, pr:$pr, step:$step, paths:$paths, claimed_at:$claimed_at, updated_at:$updated_at, ttl_seconds:$ttl_seconds}')"
        body_file="$(mktemp -t autospec-run-state.XXXXXX)"
        trap 'rm -f "$body_file"' EXIT
        {
            printf '%s\n' "$BEGIN_MARKER"
            printf '%s\n' "$state_json"
            printf '%s\n' "$END_MARKER"
        } > "$body_file"

	    comment_id="$(state_comment_id)"
	    if [ -n "$comment_id" ] && [ "$comment_id" != "null" ]; then
	        gh_api_retry "repos/$repo/issues/comments/$comment_id" -X PATCH -F "body=@$body_file" >/dev/null
	    else
	        gh issue comment "$issue" --repo "$repo" --body-file "$body_file" >/dev/null
	    fi
	    for duplicate_id in $(state_comment_ids | sed '1d'); do
	        gh_api_retry "repos/$repo/issues/comments/$duplicate_id" -X DELETE >/dev/null || true
	    done
	    printf '%s\n' "$state_json" | jq .
	    ;;
	clear)
	    for comment_id in $(state_comment_ids); do
	        gh_api_retry "repos/$repo/issues/comments/$comment_id" -X DELETE >/dev/null
	    done
	    ;;
    *)
        usage >&2
        exit 1
        ;;
esac
