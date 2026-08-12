#!/usr/bin/env bash
# Validate autospec-fleet desired-state and optional node-local config.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"

config="autospec-fleet.yml"
node_config=""
profiles_file=""
tmp_files=""

usage() {
    cat <<'EOF'
Usage: fleet-config-lint.sh --config PATH [--node-config PATH] [--profiles PATH]

Validates autospec-fleet.yml and, when supplied, ~/.autospec/fleet-node.yml.
EOF
}

cleanup() {
    [ -z "$tmp_files" ] || rm -f $tmp_files
}
trap cleanup EXIT INT TERM

fail() {
    printf 'fleet-config-lint: %s\n' "$*" >&2
    exit 2
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

yaml_to_json() {
    local src="$1"
    local dest="$2"
    if yq -o=json '.' "$src" > "$dest" 2>/dev/null; then
        return 0
    fi
    # Python yq transcodes YAML to JSON by default and forwards -o to jq.
    yq '.' "$src" > "$dest"
}

schema_validate() {
    local schema="$1"
    local src="$2"
    local json_file
    json_file="$(mktemp -t fleet-config.XXXXXX).json"
    tmp_files="$tmp_files $json_file"
    yaml_to_json "$src" "$json_file"
    ajv validate -s "$schema" --spec=draft2020 -d "$json_file" >/dev/null \
        || fail "$src failed schema validation"
}

resolve_profiles_file() {
    if [ -n "$profiles_file" ]; then
        printf '%s\n' "$profiles_file"
    elif [ -f "$repo_root/examples/model-profiles.yml" ]; then
        printf '%s\n' "$repo_root/examples/model-profiles.yml"
    elif [ -f "$HOME/.autospec/model-profiles.yml" ]; then
        printf '%s\n' "$HOME/.autospec/model-profiles.yml"
    fi
}

profile_names() {
    local file="$1"
    if [ "$(yq -r 'has("profiles")' "$file")" = "true" ]; then
        yq -r '.profiles | keys | .[]' "$file"
    else
        yq -r 'keys | .[]' "$file"
    fi
}

profile_known() {
    local file="$1"
    local profile="$2"
    profile_names "$file" | grep -Fx -- "$profile" >/dev/null
}

validate_fleet_profiles() {
    local file="$1"
    local known="$2"
    local profile

    [ -n "$known" ] || return 0
    # `[.repos[]?.profile]`, not `(.repos[]?.profile // [] | [.])`: the alternative
    # operator replaced a missing per-repo override with an empty ARRAY, which then
    # survived `select(. != null)` and was reported as `unknown profile '[]'`. A repo
    # entry without an override is the common case, so every such config failed.
    while IFS= read -r profile; do
        [ -z "$profile" ] && continue
        profile_known "$known" "$profile" \
            || fail "unknown profile '$profile' in $file"
    done <<EOF
$(yq -r '[.default_profile] + [.repos[]?.profile] | .[] | select(. != null)' "$file")
EOF
}

validate_node_profiles() {
    local file="$1"
    local known="$2"
    local profile

    [ -n "$known" ] || return 0
    while IFS= read -r profile; do
        [ -z "$profile" ] && continue
        profile_known "$known" "$profile" \
            || fail "unknown profile '$profile' in $file"
    done <<EOF
$(yq -r '.profiles[]? | select(. != null)' "$file")
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --config)
            shift
            [ $# -gt 0 ] || fail "--config requires a path"
            config="$1"
            ;;
        --config=*)
            config="${1#--config=}"
            ;;
        --node-config)
            shift
            [ $# -gt 0 ] || fail "--node-config requires a path"
            node_config="$1"
            ;;
        --node-config=*)
            node_config="${1#--node-config=}"
            ;;
        --profiles)
            shift
            [ $# -gt 0 ] || fail "--profiles requires a path"
            profiles_file="$1"
            ;;
        --profiles=*)
            profiles_file="${1#--profiles=}"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
    shift
done

need_cmd ajv
need_cmd yq

[ -f "$config" ] || fail "config not found: $config"
profiles_path="$(resolve_profiles_file)"
[ -z "$profiles_path" ] || [ -f "$profiles_path" ] || fail "profiles file not found: $profiles_path"

schema_validate "$repo_root/schemas/autospec-fleet.schema.json" "$config"
validate_fleet_profiles "$config" "$profiles_path"

if [ -n "$node_config" ]; then
    [ -f "$node_config" ] || fail "node config not found: $node_config"
    schema_validate "$repo_root/schemas/autospec-fleet-node.schema.json" "$node_config"
    validate_node_profiles "$node_config" "$profiles_path"
fi

printf 'fleet-config-lint: OK\n'
