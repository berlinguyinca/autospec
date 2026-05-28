#!/usr/bin/env bash
# Build or run per-repo autospec-run workers for an autospec fleet.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
source "$script_dir/fleet-lib.sh"

config="autospec-fleet.yml"
node_config=""
profile_override=""
parallel_override=""
dry_run=0
once=0
list_ready_bin="${AUTOSPEC_FLEET_LIST_READY:-}"

usage() {
    cat <<'EOF'
Usage: fleet-run.sh [--config PATH] [--node-config PATH] [--profile NAME]
                    [--parallel N] [--dry-run] [--once]

Loads autospec-fleet.yml, probes each eligible repository queue, and emits
per-repo /autospec-run worker commands. Dry-run mode never launches workers.
EOF
}

fail() {
    printf 'fleet-run: %s\n' "$*" >&2
    exit 2
}

positive_int() {
    case "${1:-}" in
        ''|*[!0-9]*) return 1 ;;
        *) [ "$1" -gt 0 ] ;;
    esac
}

cap_min() {
    local current="$1"
    local candidate="$2"
    if [ -z "$candidate" ] || [ "$candidate" = "null" ]; then
        printf '%s\n' "$current"
    elif [ "$candidate" -lt "$current" ]; then
        printf '%s\n' "$candidate"
    else
        printf '%s\n' "$current"
    fi
}

node_allows_profile() {
    local profile="$1"
    if [ -z "$node_config" ] || [ ! -f "$node_config" ]; then
        return 0
    fi
    yq -r '.profiles[]? // ""' "$node_config" | grep -Fx -- "$profile" >/dev/null
}

queue_has_work() {
    local repo="$1"
    local probe_json
    probe_json="$("$list_ready_bin" --repo "$repo" --batch-size 1)" || return 1
    [ "$(printf '%s\n' "$probe_json" | jq '.batch | length')" -gt 0 ]
}

while [ $# -gt 0 ]; do
    case "$1" in
        --config) shift; [ $# -gt 0 ] || fail "--config requires a path"; config="$1" ;;
        --config=*) config="${1#--config=}" ;;
        --node-config) shift; [ $# -gt 0 ] || fail "--node-config requires a path"; node_config="$1" ;;
        --node-config=*) node_config="${1#--node-config=}" ;;
        --profile) shift; [ $# -gt 0 ] || fail "--profile requires a name"; profile_override="$1" ;;
        --profile=*) profile_override="${1#--profile=}" ;;
        --parallel) shift; [ $# -gt 0 ] || fail "--parallel requires a number"; parallel_override="$1" ;;
        --parallel=*) parallel_override="${1#--parallel=}" ;;
        --list-ready-bin) shift; [ $# -gt 0 ] || fail "--list-ready-bin requires a path"; list_ready_bin="$1" ;;
        --list-ready-bin=*) list_ready_bin="${1#--list-ready-bin=}" ;;
        --dry-run) dry_run=1 ;;
        --once) once=1 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -f "$config" ] || fail "config not found: $config"
[ -z "$node_config" ] || [ -f "$node_config" ] || fail "node config not found: $node_config"
if [ -z "$list_ready_bin" ]; then
    list_ready_bin="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/list-ready-issues.sh"
fi
[ -x "$list_ready_bin" ] || list_ready_bin="$repo_root/skills/autospec-run/scripts/list-ready-issues.sh"
[ -x "$list_ready_bin" ] || fail "list-ready-issues.sh not found or not executable"

bash "$script_dir/fleet-config-lint.sh" --config "$config" ${node_config:+--node-config "$node_config"} >/dev/null
command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v yq >/dev/null 2>&1 || fail "yq is required"

capacity="$(yq -r '.parallel_repos // 1' "$config")"
positive_int "$capacity" || fail "parallel_repos must be a positive integer"
if [ -n "$node_config" ]; then
    node_cap="$(yq -r '.max_parallel_repos // ""' "$node_config")"
    positive_int "$node_cap" || fail "max_parallel_repos must be a positive integer"
    capacity="$(cap_min "$capacity" "$node_cap")"
fi
if [ -n "$parallel_override" ]; then
    positive_int "$parallel_override" || fail "--parallel must be a positive integer"
    capacity="$(cap_min "$capacity" "$parallel_override")"
fi

workspace="$(yq -r '.workspace' "$config")"
if [ -n "$node_config" ]; then
    workspace="$(yq -r ".workspace // \"$workspace\"" "$node_config")"
fi
default_profile="$(yq -r '.default_profile // ""' "$config")"
node_id="local"
[ -z "$node_config" ] || node_id="$(yq -r '.node_id // "local"' "$node_config")"

scheduled=0
repo_count="$(yq -r '.repos | length' "$config")"
idx=0
while [ "$idx" -lt "$repo_count" ]; do
    [ "$scheduled" -lt "$capacity" ] || break
    enabled="$(yq -r ".repos[$idx].enabled // true" "$config")"
    if [ "$enabled" != "true" ]; then idx=$((idx + 1)); continue; fi

    repo_url="$(yq -r ".repos[$idx].url" "$config")"
    normalized="$(normalize_repo_url "$repo_url")"
    profile="$(yq -r ".repos[$idx].profile // \"$default_profile\"" "$config")"
    [ -n "$profile_override" ] && profile="$profile_override"
    node_allows_profile "$profile" || { idx=$((idx + 1)); continue; }
    queue_has_work "$normalized" || { idx=$((idx + 1)); continue; }

    worker_id="$(fleet_worker_id "$node_id" "$normalized")"
    command="$(autospec_run_command "$profile" "$worker_id")"
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    if [ "$dry_run" -eq 1 ]; then
        printf 'fleet: %s: cd %s && %s\n' "$normalized" "$checkout_path" "$command"
    else
        printf 'fleet: launch %s: cd %s && %s\n' "$normalized" "$checkout_path" "$command"
    fi
    scheduled=$((scheduled + 1))
    idx=$((idx + 1))
done

[ "$once" -eq 0 ] || exit 0
exit 0
