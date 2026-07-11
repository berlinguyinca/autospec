#!/usr/bin/env bash
# run-state.sh — read/upsert/clear autospec-run GitHub issue state comments.

set -eu

BEGIN_MARKER="<!-- autospec-run-state:begin -->"
END_MARKER="<!-- autospec-run-state:end -->"

usage() {
    cat <<'EOF'
Usage:
  run-state.sh read                --issue <N> [--repo owner/repo]
  run-state.sh upsert              --issue <N> [--repo owner/repo] --worker-id <id> --state <state> [--step <step>] [--branch <b>] [--pr <p>] [--paths <json-or-csv>] [--ttl-seconds <N>]
  run-state.sh reconcile-linked-pr --issue <N> [--repo owner/repo] [--worker-id <id>]
  run-state.sh clear               --issue <N> [--repo owner/repo]
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


linked_open_pr_json() {
    # Return the lowest-numbered open PR whose body has a GitHub closing
    # keyword for this issue and exactly one Closeout report. `gh pr list`
    # failures are treated as no match so queue scans fail closed/non-blocking.
    prs_json="$(gh pr list \
        --repo "$repo" \
        --state open \
        --limit 100 \
        --json number,title,body,url 2>/dev/null || printf '[]\n')"
    printf '%s\n' "$prs_json" | jq -c --arg issue "$issue" '
      def close_re($n):
        "(?i)(close[sd]?|fix(e[sd])?|resolve[sd]?)\\s+#" + $n + "([^0-9]|$)";
      def closeout_count:
        [match("(?im)^##[[:space:]]+Closeout report[[:space:]]*$"; "g")] | length;
      [ .[]
        | select((.body // "") | test(close_re($issue)))
        | select(((.body // "") | closeout_count) == 1)
      ] | sort_by(.number) | .[0] // empty
    ' 2>/dev/null || true
}

reconcile_blocker_exists() {
    marker="$1"
    comments_json | jq -e --arg marker "$marker" '
      any(.[]; (.body // "") | contains($marker))
    ' >/dev/null 2>&1
}

post_reconcile_blocker() {
    pr_number="$1"
    marker="<!-- autospec-linked-pr-run-state-reconcile:pr:$pr_number -->"
    if reconcile_blocker_exists "$marker"; then
        return 0
    fi
    body_file="$(mktemp -t autospec-linked-pr-reconcile.XXXXXX)"
    {
        printf '%s\n' "$marker"
        printf 'Autospec run-state reconciliation found linked PR #%s with one Closeout report while issue #%s was still in `claimed` state with no recorded PR. Resume post-PR handoff from PR #%s: run review/merge gates or comment the blocking gate failure, then release or merge the claim.\n' "$pr_number" "$issue" "$pr_number"
    } > "$body_file"
    gh issue comment "$issue" --repo "$repo" --body-file "$body_file" >/dev/null 2>&1 || true
    rm -f "$body_file"
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

	    # Telemetry (issue #1772): fire-and-forget session emit after the
	    # upsert. No prior state comment -> session.started; a prior
	    # comment being overwritten -> session.step. Guarded source: an
	    # absent shim/binary/DSN is a silent no-op and never alters this
	    # command's exit status or output.
	    _RS_H="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
	    if [ -f "$_RS_H/emit-event.sh" ]; then
	        # shellcheck source=/dev/null
	        . "$_RS_H/emit-event.sh"
	        if [ -z "$existing" ]; then
	            emit_event session.started repo="$repo" issue="$issue" step="$step" || true
	        else
	            emit_event session.step repo="$repo" issue="$issue" step="$step" || true
	        fi
	    fi

	    printf '%s\n' "$state_json" | jq .
	    ;;

    reconcile-linked-pr)
        current_json="$($0 read --issue "$issue" --repo "$repo" 2>/dev/null || true)"
        if [ -z "$current_json" ]; then
            jq -n --argjson issue "$issue" --arg repo "$repo" --arg reason "missing_run_state" \
                '{reconciled:false, issue:$issue, repo:$repo, reason:$reason}'
            exit 0
        fi
        current_pr="$(printf '%s\n' "$current_json" | jq -r '.pr // ""' 2>/dev/null || true)"
        if [ -n "$current_pr" ]; then
            jq -n --argjson issue "$issue" --arg repo "$repo" --arg pr "$current_pr" --arg reason "pr_already_recorded" \
                '{reconciled:false, issue:$issue, repo:$repo, pr:$pr, reason:$reason}'
            exit 0
        fi
        current_worker="$(printf '%s\n' "$current_json" | jq -r '.worker_id // empty' 2>/dev/null || true)"
        [ -n "$worker_id" ] || worker_id="$current_worker"
        [ -n "$worker_id" ] || worker_id="reconcile-linked-pr"
        current_branch="$(printf '%s\n' "$current_json" | jq -r '.branch // ""' 2>/dev/null || true)"
        current_paths="$(printf '%s\n' "$current_json" | jq -c '.paths // []' 2>/dev/null || printf '[]')"
        current_ttl="$(printf '%s\n' "$current_json" | jq -r '.ttl_seconds // 10800' 2>/dev/null || printf '10800')"
        case "$current_ttl" in *[!0-9]*|'') current_ttl=10800 ;; esac

        pr_json="$(linked_open_pr_json)"
        if [ -z "$pr_json" ]; then
            jq -n --argjson issue "$issue" --arg repo "$repo" --arg reason "no_linked_pr_with_one_closeout" \
                '{reconciled:false, issue:$issue, repo:$repo, reason:$reason}'
            exit 0
        fi
        pr_number="$(printf '%s\n' "$pr_json" | jq -r '.number')"

        # Record the PR/step first. Any subsequent labels or handoff recovery
        # sees a non-empty `.pr` plus a fresh `.updated_at` in the authoritative
        # run-state comment.
        updated_json="$($0 upsert \
            --issue "$issue" \
            --repo "$repo" \
            --worker-id "$worker_id" \
            --state claimed \
            --step post_pr_handoff_failed \
            --branch "$current_branch" \
            --pr "$pr_number" \
            --paths "$current_paths" \
            --ttl-seconds "$current_ttl")"
        post_reconcile_blocker "$pr_number"
        printf '%s\n' "$updated_json" | jq --argjson reconciled true --argjson pr "$pr_number" \
            '. + {reconciled:$reconciled, pr:($pr|tostring)}'
        ;;
	clear)
	    for comment_id in $(state_comment_ids); do
	        gh_api_retry "repos/$repo/issues/comments/$comment_id" -X DELETE >/dev/null
	    done

	    # Telemetry (issue #1772): fire-and-forget terminal emit after
	    # clear. Guarded source: an absent shim/binary/DSN is a silent
	    # no-op and never alters this command's exit status or output.
	    _RS_H="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
	    if [ -f "$_RS_H/emit-event.sh" ]; then
	        # shellcheck source=/dev/null
	        . "$_RS_H/emit-event.sh"
	        emit_event session.terminal repo="$repo" issue="$issue" step="" || true
	    fi
	    ;;
    *)
        usage >&2
        exit 1
        ;;
esac
