#!/usr/bin/env bash
# scripts/autonomous-integration-branch.sh — manage the long-lived autonomous integration branch.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$script_dir/autospec-runtime-config.sh" ]; then
    # shellcheck source=scripts/autospec-runtime-config.sh
    . "$script_dir/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

COMMAND="${1:-}"
[ $# -gt 0 ] && shift || true

PARENT="main"
REPO=""
TAKEOVER=0

usage() {
    cat <<EOF
Usage: $0 <ensure|sync|reset|status> --parent <branch> [--repo <owner/repo>] [--takeover]

Subcommands:
  ensure  Create or reuse autospec/autonomous-<parent>, push it, and write .autospec/explore-mode.json.
  sync    Merge the parent tip into the integration branch; conflicts abort with exit 65.
  reset   Recreate the integration branch from the parent tip and push it (requires the
          rollup PR to be merged or absent).
  status  Emit JSON with rollup PR state, accumulated PR count, age days, and diff lines.

Flags:
  --parent <branch>     Parent branch this integration branch tracks (default: main).
  --repo <owner/repo>   Repo slug for gh calls (default: derived from origin remote).
  --takeover            ensure only: overwrite a conflicting .autospec/explore-mode.json.
EOF
}

err() { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --parent)
            shift
            [ $# -gt 0 ] || { err "--parent requires a value"; exit 2; }
            PARENT="$1"
            ;;
        --parent=*)
            PARENT="${1#--parent=}"
            ;;
        --repo)
            shift
            [ $# -gt 0 ] || { err "--repo requires a value"; exit 2; }
            REPO="$1"
            ;;
        --repo=*)
            REPO="${1#--repo=}"
            ;;
        --takeover)
            TAKEOVER=1
            ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            err "unknown arg: $1"; usage; exit 2 ;;
    esac
    shift
done

case "$COMMAND" in
    ensure|sync|reset|status) ;;
    -h|--help) usage; exit 0 ;;
    *) err "unknown subcommand: ${COMMAND:-<none>}"; usage; exit 2 ;;
esac

if [ -z "$PARENT" ]; then
    err "--parent is required"
    exit 2
fi

config_get() {
    local key="$1" default="$2"
    if command -v autospec_runtime_config_get >/dev/null 2>&1; then
        autospec_runtime_config_get "$key" "$default"
    else
        printf '%s\n' "$default"
    fi
}

json_escape() {
    python3 - "$1" <<'PY'
import json
import sys
print(json.dumps(sys.argv[1]))
PY
}

status_probe_failed() {
    err "status probe failed: $*"
    exit 1
}

require_json_array() {
    local label="$1" payload="$2"
    printf '%s' "$payload" | jq -e 'type == "array"' >/dev/null \
        || status_probe_failed "$label returned invalid JSON"
}

repo_root() {
    git rev-parse --show-toplevel
}

repo_slug() {
    if [ -n "$REPO" ]; then
        printf '%s\n' "$REPO"
        return 0
    fi
    if [ -n "${AUTOSPEC_REPO:-}" ]; then
        printf '%s\n' "$AUTOSPEC_REPO"
        return 0
    fi
    local remote
    remote="$(git config --get remote.origin.url 2>/dev/null || true)"
    case "$remote" in
        git@github.com:*)
            remote="${remote#git@github.com:}"
            remote="${remote%.git}"
            printf '%s\n' "$remote"
            return 0
            ;;
        https://github.com/*)
            remote="${remote#https://github.com/}"
            remote="${remote%.git}"
            printf '%s\n' "$remote"
            return 0
            ;;
    esac
    err "cannot derive repo slug; pass --repo"
    exit 2
}

parent_ref() {
    case "$PARENT" in
        origin/*) printf '%s\n' "$PARENT" ;;
        *)        printf 'origin/%s\n' "$PARENT" ;;
    esac
}

integration_branch() {
    local prefix
    prefix="$(config_get "autonomous.self_originated.integration_branch_prefix" "autospec/autonomous-")"
    printf '%s%s\n' "$prefix" "${PARENT#origin/}"
}

branch_exists() {
    local branch="$1"
    git show-ref --verify --quiet "refs/heads/$branch" \
        || git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1
}

# Always refreshes the local branch ref from origin when the remote branch exists,
# so stale local state is never used (finding: stale local branch).
ensure_local_branch() {
    local branch="$1"
    if git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
        git fetch origin "$branch:$branch" --force --quiet
        return 0
    fi
    git show-ref --verify --quiet "refs/heads/$branch"
}

fetch_parent_tip() {
    git fetch origin "$PARENT" --quiet || {
        err "code_health:autonomous_integration_parent_fetch_failed parent=${PARENT#origin/}"
        exit 1
    }
}

write_mode_file() {
    local branch="$1" slug="$2" base="$3" head_sha="$4" root mode_file tmp_file
    root="$(repo_root)"
    mode_file="$root/.autospec/explore-mode.json"
    tmp_file="$mode_file.tmp"
    mkdir -p "$(dirname "$mode_file")"
    {
        printf '{\n'
        printf '  "branch": %s,\n' "$(json_escape "$branch")"
        printf '  "slug": %s,\n' "$(json_escape "$slug")"
        printf '  "base": %s,\n' "$(json_escape "$base")"
        printf '  "head_sha": %s,\n' "$(json_escape "$head_sha")"
        printf '  "kind": "integration"\n'
        printf '}\n'
    } > "$tmp_file"
    mv "$tmp_file" "$mode_file"
}

# Refuses to clobber an existing mode file that records a different branch or a
# non-integration kind, unless --takeover was passed. Mirrors explore-sandbox.sh:81-97.
check_mode_file_conflict() {
    local branch="$1" root mode_file existing_branch existing_kind
    root="$(repo_root)"
    mode_file="$root/.autospec/explore-mode.json"
    [ -f "$mode_file" ] || return 0

    existing_branch="$(jq -r '.branch // empty' "$mode_file" 2>/dev/null || printf '')"
    existing_kind="$(jq -r '.kind // empty' "$mode_file" 2>/dev/null || printf '')"

    if [ "$existing_branch" = "$branch" ] && [ "$existing_kind" = "integration" ]; then
        return 0
    fi
    if [ "$TAKEOVER" -eq 1 ]; then
        return 0
    fi
    err "code_health:integration_mode_conflict existing_branch=${existing_branch:-<none>} existing_kind=${existing_kind:-<none>} requested_branch=$branch"
    exit 6
}

# --- dedicated temporary worktree: sync/reset never touch the caller's checkout ---

CURRENT_TMP_WT=""
cleanup_tmp_worktree() {
    if [ -n "$CURRENT_TMP_WT" ] && [ -d "$CURRENT_TMP_WT" ]; then
        git worktree remove --force "$CURRENT_TMP_WT" >/dev/null 2>&1 || true
        rm -rf "$CURRENT_TMP_WT"
    fi
    CURRENT_TMP_WT=""
}
trap cleanup_tmp_worktree EXIT

with_integration_worktree() {
    local start_ref="$1" op_fn="$2" rc
    CURRENT_TMP_WT="$(mktemp -d "${TMPDIR:-/tmp}/autonomous-integration.XXXXXX")"

    if ! git worktree add --detach "$CURRENT_TMP_WT" "$start_ref" >/dev/null 2>&1; then
        err "code_health:autonomous_integration_worktree_failed ref=$start_ref"
        cleanup_tmp_worktree
        return 1
    fi

    set +e
    ( cd "$CURRENT_TMP_WT" && "$op_fn" )
    rc=$?
    set -e

    cleanup_tmp_worktree
    return "$rc"
}

# Runs inside the temp worktree (detached at the integration branch tip).
op_sync() {
    if ! git merge --no-edit "$pref"; then
        git merge --abort >/dev/null 2>&1 || true
        return 65
    fi
    git push origin "HEAD:$branch"
}

# Runs inside the temp worktree (detached at the parent tip).
op_reset() {
    git push origin ":$branch" || true
    git push origin "HEAD:$branch"
}

cmd_ensure() {
    local branch pref slug head_sha
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    fetch_parent_tip
    if branch_exists "$branch"; then
        info "integration branch already exists: $branch (reusing)"
        ensure_local_branch "$branch"
    else
        info "creating integration branch: $branch off $pref"
        git branch "$branch" "$pref"
        git push -u origin "$branch"
    fi

    check_mode_file_conflict "$branch"

    head_sha="$(git rev-parse "$branch")"
    write_mode_file "$branch" "$slug" "${PARENT#origin/}" "$head_sha"
    info "wrote .autospec/explore-mode.json"
}

cmd_sync() {
    local branch pref slug head_sha rc
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    fetch_parent_tip
    branch_exists "$branch" || {
        err "code_health:autonomous_integration_branch_missing branch=$branch"
        exit 1
    }
    ensure_local_branch "$branch"

    set +e
    with_integration_worktree "$branch" op_sync
    rc=$?
    set -e

    if [ "$rc" -eq 65 ]; then
        err "code_health:autonomous_integration_merge_conflict branch=$branch parent=${PARENT#origin/}"
        exit 65
    fi
    [ "$rc" -eq 0 ] || exit "$rc"

    ensure_local_branch "$branch"
    head_sha="$(git rev-parse "$branch")"
    write_mode_file "$branch" "$slug" "${PARENT#origin/}" "$head_sha"
}

cmd_reset() {
    local branch pref slug rollup_open_json rc head_sha
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    fetch_parent_tip
    ensure_local_branch "$branch" || true

    rollup_open_json="$(gh pr list --repo "$slug" --head "$branch" --base "${PARENT#origin/}" --state open --json number 2>/dev/null || printf '[]')"
    if ! printf '%s' "$rollup_open_json" | jq -e 'type == "array" and length == 0' >/dev/null 2>&1; then
        err "code_health:integration_reset_rollup_open branch=$branch"
        exit 7
    fi

    set +e
    with_integration_worktree "$pref" op_reset
    rc=$?
    set -e
    [ "$rc" -eq 0 ] || exit "$rc"

    ensure_local_branch "$branch"
    head_sha="$(git rev-parse "$branch")"
    write_mode_file "$branch" "$slug" "${PARENT#origin/}" "$head_sha"
}

status_rollup_json() {
    local slug="$1" branch="$2" parent="$3" open_json
    open_json="$(gh pr list --repo "$slug" --head "$branch" --base "$parent" --state open --json number,state)" \
        || status_probe_failed "rollup PR query failed"
    require_json_array "rollup PR query" "$open_json"
    if printf '%s' "$open_json" | jq -e 'length > 0' >/dev/null 2>&1; then
        printf '%s' "$open_json"
        return 0
    fi
    gh pr list --repo "$slug" --head "$branch" --base "$parent" --state merged --json number,state \
        || status_probe_failed "rollup PR query failed"
}

status_accumulated_pr_count() {
    local pref="$1" branch="$2" count
    count="$(git rev-list --count --merges "$pref..$branch")" \
        || status_probe_failed "accumulated PR count query failed"
    case "$count" in *[!0-9]*|'') status_probe_failed "accumulated PR count returned non-numeric value" ;; esac
    printf '%s\n' "$count"
}

status_age_days() {
    local pref="$1" branch="$2" age_epoch now_epoch age_days rc
    set +e
    age_epoch="$(git log --format=%ct "$pref..$branch" 2>/dev/null | tail -1)"
    rc=$?
    set -e
    [ "$rc" -eq 0 ] || status_probe_failed "branch age query failed"
    if [ -z "$age_epoch" ]; then
        age_epoch="$(git log -1 --format=%ct "$branch")" || status_probe_failed "branch age query failed"
    fi
    now_epoch="$(date -u +%s)"
    case "$age_epoch" in *[!0-9]*|'') status_probe_failed "branch age query returned non-numeric epoch" ;; esac
    age_days=$(( (now_epoch - age_epoch) / 86400 ))
    [ "$age_days" -ge 0 ] || age_days=0
    printf '%s\n' "$age_days"
}

status_diff_lines() {
    local pref="$1" branch="$2" diff_lines
    diff_lines="$(git diff --numstat "$pref...$branch" | awk '{a+=$1; d+=$2} END {print a+d+0}')" \
        || status_probe_failed "branch diff query failed"
    case "$diff_lines" in *[!0-9]*|'') status_probe_failed "branch diff query returned non-numeric line count" ;; esac
    printf '%s\n' "$diff_lines"
}

cmd_status() {
    local branch pref slug rollup_json first_pr first_state accumulated_pr_count
    local age_days diff_lines first_state_json
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    branch_exists "$branch" || status_probe_failed "integration branch not found: $branch"
    ensure_local_branch "$branch" || status_probe_failed "could not fetch integration branch: $branch"

    rollup_json="$(status_rollup_json "$slug" "$branch" "${PARENT#origin/}")"
    first_pr="$(printf '%s' "$rollup_json" | jq -r 'if length > 0 then (sort_by(.number) | reverse | .[0].number) else null end')" \
        || status_probe_failed "rollup PR number parse failed"
    first_state="$(printf '%s' "$rollup_json" | jq -r 'if length > 0 then (sort_by(.number) | reverse | .[0].state) else null end')" \
        || status_probe_failed "rollup PR state parse failed"

    accumulated_pr_count="$(status_accumulated_pr_count "$pref" "$branch")"
    age_days="$(status_age_days "$pref" "$branch")"
    diff_lines="$(status_diff_lines "$pref" "$branch")"

    if [ "$first_pr" = "null" ]; then
        first_state_json="null"
    else
        first_state_json="$(json_escape "$first_state")"
    fi

    printf '{\n'
    printf '  "branch": %s,\n' "$(json_escape "$branch")"
    printf '  "rollup_pr": {"number": %s, "state": %s},\n' "$first_pr" "$first_state_json"
    printf '  "accumulated_pr_count": %s,\n' "$accumulated_pr_count"
    printf '  "age_days": %s,\n' "$age_days"
    printf '  "diff_lines": %s\n' "$diff_lines"
    printf '}\n'
}

case "$COMMAND" in
    ensure) cmd_ensure ;;
    sync)   cmd_sync ;;
    reset)  cmd_reset ;;
    status) cmd_status ;;
esac
