#!/usr/bin/env bash
# scripts/autonomous-integration-branch.sh — manage the long-lived autonomous integration branch.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=scripts/autospec-runtime-config.sh
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

COMMAND="${1:-}"
[ $# -gt 0 ] && shift || true

PARENT="main"
REPO=""

usage() {
    cat <<EOF
Usage: $0 <ensure|sync|reset|status> --parent <branch> [--repo <owner/repo>]

Subcommands:
  ensure  Create or reuse autospec/autonomous-<parent>, push it, and write .autospec/explore-mode.json.
  sync    Merge the parent tip into the integration branch; conflicts abort with exit 65.
  reset   Recreate the integration branch from the parent tip and push it.
  status  Emit JSON with rollup PR state, accumulated PR count, age days, and diff lines.
EOF
}

err() { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --parent)      shift; PARENT="${1:-main}" ;;
        --parent=*)    PARENT="${1#--parent=}" ;;
        --repo)        shift; REPO="${1:-}" ;;
        --repo=*)      REPO="${1#--repo=}" ;;
        -h|--help)     usage; exit 0 ;;
        *)             err "unknown arg: $1"; usage; exit 2 ;;
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
    local remote=""
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
    printf '%s\n' "berlinguyinca/autospec"
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

ensure_local_branch() {
    local branch="$1"
    if git show-ref --verify --quiet "refs/heads/$branch"; then
        return 0
    fi
    if git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
        git fetch origin "$branch:$branch" --quiet
    fi
}

write_mode_file() {
    local branch="$1" slug="$2" base="$3" head_sha="$4" root mode_file
    root="$(repo_root)"
    mode_file="$root/.autospec/explore-mode.json"
    mkdir -p "$(dirname "$mode_file")"
    {
        printf '{\n'
        printf '  "branch": %s,\n' "$(json_escape "$branch")"
        printf '  "slug": %s,\n' "$(json_escape "$slug")"
        printf '  "base": %s,\n' "$(json_escape "$base")"
        printf '  "head_sha": %s,\n' "$(json_escape "$head_sha")"
        printf '  "kind": "integration"\n'
        printf '}\n'
    } > "$mode_file"
}

cmd_ensure() {
    local branch pref slug head_sha
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    git fetch origin "$PARENT" --quiet || true
    if branch_exists "$branch"; then
        info "integration branch already exists: $branch (reusing)"
        ensure_local_branch "$branch"
    else
        info "creating integration branch: $branch off $pref"
        git branch "$branch" "$pref"
        git push -u origin "$branch"
    fi

    head_sha="$(git rev-parse "$branch")"
    write_mode_file "$branch" "$slug" "${PARENT#origin/}" "$head_sha"
    info "wrote .autospec/explore-mode.json"
}

cmd_sync() {
    local branch pref
    branch="$(integration_branch)"
    pref="$(parent_ref)"

    git fetch origin "$PARENT" --quiet || true
    ensure_local_branch "$branch"
    git checkout "$branch"
    if ! git merge --no-edit "$pref"; then
        git merge --abort >/dev/null 2>&1 || true
        err "code_health:autonomous_integration_merge_conflict branch=$branch parent=${PARENT#origin/}"
        exit 65
    fi
    git push -u origin "$branch"
}

cmd_reset() {
    local branch pref
    branch="$(integration_branch)"
    pref="$(parent_ref)"

    git fetch origin "$PARENT" --quiet || true
    git branch -f "$branch" "$pref"
    git push -u origin "$branch"
}

cmd_status() {
    local branch pref slug pr_json first_pr first_state pr_count age_epoch now_epoch age_days diff_lines
    branch="$(integration_branch)"
    pref="$(parent_ref)"
    slug="$(repo_slug)"

    pr_json="$(gh pr list --repo "$slug" --head "$branch" --base "${PARENT#origin/}" --state all --json number,state 2>/dev/null || echo '[]')"
    first_pr="$(printf '%s' "$pr_json" | jq -r 'if type=="array" and length>0 then .[0].number else null end' 2>/dev/null || printf 'null')"
    first_state="$(printf '%s' "$pr_json" | jq -r 'if type=="array" and length>0 then .[0].state else null end' 2>/dev/null || printf 'null')"
    pr_count="$(printf '%s' "$pr_json" | jq -r 'if type=="array" then length else 0 end' 2>/dev/null || printf '0')"

    age_epoch="$(git log -1 --format=%ct "$branch" 2>/dev/null || printf '0')"
    now_epoch="$(date -u +%s)"
    case "$age_epoch" in *[!0-9]*|'') age_epoch="$now_epoch" ;; esac
    age_days=$(( (now_epoch - age_epoch) / 86400 ))
    [ "$age_days" -ge 0 ] || age_days=0

    diff_lines="$(git diff "$pref...$branch" 2>/dev/null | wc -l | tr -d ' ')"
    case "$diff_lines" in *[!0-9]*|'') diff_lines=0 ;; esac

    if [ "$first_pr" = "null" ]; then
        first_state_json="null"
    else
        first_state_json="$(json_escape "$first_state")"
    fi

    printf '{\n'
    printf '  "branch": %s,\n' "$(json_escape "$branch")"
    printf '  "rollup_pr": {"number": %s, "state": %s},\n' "$first_pr" "$first_state_json"
    printf '  "accumulated_pr_count": %s,\n' "$pr_count"
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
