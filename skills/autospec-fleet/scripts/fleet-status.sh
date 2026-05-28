#!/usr/bin/env bash
# Summarize autospec-fleet repository queue state.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
source "$script_dir/fleet-lib.sh"

config="autospec-fleet.yml"
json_output=0
list_ready_bin="${AUTOSPEC_FLEET_LIST_READY:-}"

usage() {
    cat <<'EOF'
Usage: fleet-status.sh [--config PATH] [--json] [--list-ready-bin PATH]
EOF
}

fail() {
    printf 'fleet-status: %s\n' "$*" >&2
    exit 2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --config) shift; [ $# -gt 0 ] || fail "--config requires a path"; config="$1" ;;
        --config=*) config="${1#--config=}" ;;
        --json) json_output=1 ;;
        --list-ready-bin) shift; [ $# -gt 0 ] || fail "--list-ready-bin requires a path"; list_ready_bin="$1" ;;
        --list-ready-bin=*) list_ready_bin="${1#--list-ready-bin=}" ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -f "$config" ] || fail "config not found: $config"
if [ -z "$list_ready_bin" ]; then
    list_ready_bin="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/list-ready-issues.sh"
fi
[ -x "$list_ready_bin" ] || list_ready_bin="$repo_root/skills/autospec-run/scripts/list-ready-issues.sh"
[ -x "$list_ready_bin" ] || fail "list-ready-issues.sh not found or not executable"

bash "$script_dir/fleet-config-lint.sh" --config "$config" >/dev/null
command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v yq >/dev/null 2>&1 || fail "yq is required"

workspace="$(yq -r '.workspace' "$config")"
repos_json="[]"
repo_count="$(yq -r '.repos | length' "$config")"
idx=0
while [ "$idx" -lt "$repo_count" ]; do
    enabled="$(yq -r ".repos[$idx].enabled // true" "$config")"
    repo_url="$(yq -r ".repos[$idx].url" "$config")"
    normalized="$(normalize_repo_url "$repo_url")"
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    if [ "$enabled" = "true" ]; then
        probe_json="$("$list_ready_bin" --repo "$normalized" --batch-size 1)"
    else
        probe_json='{"ready":[],"blocked":[],"claimed":[],"conflicts":[],"batch":[]}'
    fi
    repo_obj="$(printf '%s\n' "$probe_json" | jq \
        --arg repo "$normalized" \
        --arg path "$checkout_path" \
        --argjson enabled "$enabled" \
        '{repo:$repo,path:$path,enabled:$enabled,ready:(.ready|length),blocked:(.blocked|length),claimed:(.claimed|length),conflicts:(.conflicts|length),batch:(.batch|length)}')"
    repos_json="$(jq --argjson repo "$repo_obj" '. + [$repo]' <<<"$repos_json")"
    idx=$((idx + 1))
done

if [ "$json_output" -eq 1 ]; then
    jq -n --argjson repos "$repos_json" '{repos:$repos}'
else
    jq -r '.[] | "fleet: \(.repo) ready=\(.ready) blocked=\(.blocked) claimed=\(.claimed)"' <<<"$repos_json"
fi
