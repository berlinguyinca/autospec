#!/usr/bin/env bash
# Forward autospec-stop to active autospec-fleet repository checkouts.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
source "$script_dir/fleet-lib.sh"

config="autospec-fleet.yml"
mode="--graceful"
stop_bin="${AUTOSPEC_FLEET_STOP_BIN:-}"

usage() {
    cat <<'EOF'
Usage: fleet-stop.sh [--config PATH] [--graceful|--immediate] [--stop-bin PATH]
EOF
}

fail() {
    printf 'fleet-stop: %s\n' "$*" >&2
    exit 2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --config) shift; [ $# -gt 0 ] || fail "--config requires a path"; config="$1" ;;
        --config=*) config="${1#--config=}" ;;
        --graceful) mode="--graceful" ;;
        --immediate) mode="--immediate" ;;
        --stop-bin) shift; [ $# -gt 0 ] || fail "--stop-bin requires a path"; stop_bin="$1" ;;
        --stop-bin=*) stop_bin="${1#--stop-bin=}" ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -f "$config" ] || fail "config not found: $config"
if [ -z "$stop_bin" ]; then
    stop_bin="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh"
fi
[ -x "$stop_bin" ] || stop_bin="$repo_root/scripts/autospec-stop.sh"
[ -x "$stop_bin" ] || fail "autospec-stop.sh not found or not executable"

bash "$script_dir/fleet-config-lint.sh" --config "$config" >/dev/null
command -v yq >/dev/null 2>&1 || fail "yq is required"

workspace="$(yq -r '.workspace' "$config")"
stopped=0
repo_count="$(yq -r '.repos | length' "$config")"
idx=0
while [ "$idx" -lt "$repo_count" ]; do
    enabled="$(yq -r ".repos[$idx].enabled != false" "$config")"
    if [ "$enabled" != "true" ]; then idx=$((idx + 1)); continue; fi
    repo_url="$(yq -r ".repos[$idx].url" "$config")"
    normalized="$(normalize_repo_url "$repo_url")"
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    if [ -d "$checkout_path" ]; then
        ( cd "$checkout_path" && bash "$stop_bin" "$mode" )
        printf 'fleet-stop: %s %s\n' "$normalized" "$mode"
        stopped=$((stopped + 1))
    fi
    idx=$((idx + 1))
done
printf 'fleet-stop: stopped=%s\n' "$stopped"
