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
queue_bin="${AUTOSPEC_FLEET_QUEUE_BIN:-${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-}}}"

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
    probe_json="$("$queue_bin" queue ready --repo "$repo" --batch-size 1)" || return 1
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
        --queue-bin) shift; [ $# -gt 0 ] || fail "--queue-bin requires a path"; queue_bin="$1" ;;
        --queue-bin=*) queue_bin="${1#--queue-bin=}" ;;
        --dry-run) dry_run=1 ;;
        --once) once=1 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -f "$config" ] || fail "config not found: $config"
[ -z "$node_config" ] || [ -f "$node_config" ] || fail "node config not found: $node_config"
if [ -z "$queue_bin" ] && [ -x "$repo_root/target/debug/autospec" ]; then
    queue_bin="$repo_root/target/debug/autospec"
fi
if [ -z "$queue_bin" ] && command -v autospec >/dev/null 2>&1; then
    queue_bin="$(command -v autospec)"
fi
[ -n "$queue_bin" ] && { [ -x "$queue_bin" ] || command -v "$queue_bin" >/dev/null 2>&1; } \
    || fail "autospec queue binary not found or not executable"

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
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    display_command="$(fleet_worker_command "$profile" "$worker_id" "$normalized" "$checkout_path")"

    if [ "$dry_run" -eq 1 ]; then
        # Dry-run is a pure preview: it never touches the filesystem (no
        # checkout-existence check, no heartbeat read/write) and never
        # spawns anything.
        printf 'fleet: %s: cd %s && %s\n' "$normalized" "$checkout_path" "$display_command"
    elif [ ! -d "$checkout_path" ]; then
        # A missing checkout means "clone this repo first" — spawning a
        # conductor against a directory that doesn't exist would be worse
        # than skipping it (a broken worker instead of no worker), so skip
        # with a clear message and let the next fleet-run pick it up once
        # the checkout exists.
        printf 'fleet: %s: checkout not found at %s; skipping launch\n' "$normalized" "$checkout_path"
    elif fleet_worker_live "$normalized"; then
        # Idempotence: never start a second conductor for a repo that
        # already has one. Liveness is decided by the shared
        # process-heartbeats store (see fleet_worker_live in fleet-lib.sh),
        # never a PID guess or a `pgrep` on a command string.
        printf 'fleet: %s: worker already live; skipping\n' "$normalized"
    else
        autonomous_bin=""
        autonomous_bin="$(fleet_autonomous_bin)" || autonomous_bin=""
        if [ -z "$autonomous_bin" ]; then
            printf 'code_health:fleet_worker_spawn_failed repo=%s reason=autospec-autonomous-not-found\n' "$normalized" >&2
        elif "$autonomous_bin" start --detach --repo-dir "$checkout_path" --repo "$normalized"; then
            # A single repo's spawn failure must never abort the fleet — the
            # whole script runs under `set -euo pipefail`, so this branch is
            # a deliberate if/then, never a one-sided `&&`.
            fleet_worker_mark_live "$normalized" "$worker_id" || true
            printf 'fleet: launch %s: cd %s && %s\n' "$normalized" "$checkout_path" "$display_command"
        else
            printf 'code_health:fleet_worker_spawn_failed repo=%s\n' "$normalized" >&2
        fi
    fi
    scheduled=$((scheduled + 1))
    idx=$((idx + 1))
done

[ "$once" -eq 0 ] || exit 0
exit 0
