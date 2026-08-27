#!/usr/bin/env bash
# Summarize autospec-fleet repository queue state.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
source "$script_dir/fleet-lib.sh"

config="autospec-fleet.yml"
json_output=0
queue_bin="${AUTOSPEC_FLEET_QUEUE_BIN:-${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-}}}"

usage() {
    cat <<'EOF'
Usage: fleet-status.sh [--config PATH] [--json] [--queue-bin PATH]
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
        --queue-bin) shift; [ $# -gt 0 ] || fail "--queue-bin requires a path"; queue_bin="$1" ;;
        --queue-bin=*) queue_bin="${1#--queue-bin=}" ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -f "$config" ] || fail "config not found: $config"
if [ -z "$queue_bin" ] && [ -x "$repo_root/target/debug/autospec" ]; then
    queue_bin="$repo_root/target/debug/autospec"
fi
if [ -z "$queue_bin" ] && command -v autospec >/dev/null 2>&1; then
    queue_bin="$(command -v autospec)"
fi
[ -n "$queue_bin" ] && { [ -x "$queue_bin" ] || command -v "$queue_bin" >/dev/null 2>&1; } \
    || fail "autospec queue binary not found or not executable"

bash "$script_dir/fleet-config-lint.sh" --config "$config" >/dev/null
command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v yq >/dev/null 2>&1 || fail "yq is required"

workspace="$(yq -r '.workspace' "$config")"
repos_json="[]"
repo_count="$(yq -r '.repos | length' "$config")"
idx=0
while [ "$idx" -lt "$repo_count" ]; do
    enabled="$(yq -r ".repos[$idx].enabled != false" "$config")"
    repo_url="$(yq -r ".repos[$idx].url" "$config")"
    normalized="$(normalize_repo_url "$repo_url")"
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    if [ "$enabled" = "true" ]; then
        probe_json="$("$queue_bin" queue ready --repo "$normalized" --batch-size 1)"
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
