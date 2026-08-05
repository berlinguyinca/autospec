#!/usr/bin/env bats
# tests/unit/test_autospec_fleet_config.bats - autospec-fleet config linting.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-config-lint.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-config-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

write_config() {
    cat > "$1"
}

@test "fleet-config-lint accepts examples/fleet.yml" {
    run bash "$LINT" --config "$REPO_ROOT/examples/fleet.yml"

    [ "$status" -eq 0 ]
    [[ "$output" == *"fleet-config-lint: OK"* ]]
}

@test "fleet-config-lint accepts node-local capacity config" {
    node="$TEST_TMPDIR/fleet-node.yml"
    cat > "$node" <<'YAML'
node_id: mac-mini-01
workspace: ~/.autospec/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-6-35b-a3b-laptop
YAML

    run bash "$LINT" --config "$REPO_ROOT/examples/fleet.yml" --node-config "$node"

    [ "$status" -eq 0 ]
}

@test "fleet-config-lint rejects missing required fields" {
    config="$TEST_TMPDIR/missing-required.yml"
    write_config "$config" <<'YAML'
version: 1
workspace: .autospec-fleet/repos
YAML

    run bash "$LINT" --config "$config"

    [ "$status" -eq 2 ]
    [[ "$output" == *"failed schema validation"* ]]
}

@test "fleet-config-lint rejects unsupported repo URLs" {
    config="$TEST_TMPDIR/bad-url.yml"
    write_config "$config" <<'YAML'
version: 1
workspace: .autospec-fleet/repos
repos:
  - url: https://example.com/org/repo.git
    profile: qwen3-6-35b-a3b-laptop
YAML

    run bash "$LINT" --config "$config"

    [ "$status" -eq 2 ]
    [[ "$output" == *"failed schema validation"* ]]
}

@test "fleet-config-lint rejects unknown profile references" {
    config="$TEST_TMPDIR/unknown-profile.yml"
    write_config "$config" <<'YAML'
version: 1
workspace: .autospec-fleet/repos
default_profile: missing-laptop
repos:
  - url: https://github.com/org/repo.git
    profile: qwen3-6-35b-a3b-laptop
YAML

    run bash "$LINT" --config "$config"

    [ "$status" -eq 2 ]
    [[ "$output" == *"unknown profile 'missing-laptop'"* ]]
}

@test "fleet-config-lint rejects invalid concurrency fields" {
    config="$TEST_TMPDIR/bad-concurrency.yml"
    write_config "$config" <<'YAML'
version: 1
workspace: .autospec-fleet/repos
parallel_repos: 0
repos:
  - url: https://github.com/org/repo.git
YAML

    run bash "$LINT" --config "$config"

    [ "$status" -eq 2 ]
    [[ "$output" == *"failed schema validation"* ]]
}
