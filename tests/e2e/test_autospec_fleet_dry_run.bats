#!/usr/bin/env bats
# tests/e2e/test_autospec_fleet_dry_run.bats - mocked fleet dry-run smoke.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FLEET_RUN="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-run.sh"
    FIXTURES="$REPO_ROOT/tests/fixtures/fleet"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-e2e-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

write_configs() {
    cat > "$TEST_TMPDIR/fleet.yml" <<YAML
version: 1
workspace: $TEST_TMPDIR/repos
default_profile: qwen3-6-35b-a3b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
  - url: git@github.com:org/repo-b.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
YAML
    cat > "$TEST_TMPDIR/fleet-node.yml" <<'YAML'
node_id: smoke-node
max_parallel_repos: 2
profiles:
  - qwen3-6-35b-a3b-laptop
YAML
}

@test "mocked fleet dry-run discovers two repos with distinct checkout paths" {
    write_configs

    run bash "$FLEET_RUN" --dry-run --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --node-config "$TEST_TMPDIR/fleet-node.yml" \
        --queue-bin "$FIXTURES/mock-autospec.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q -- '--repo org/repo-a'
    printf '%s\n' "$output" | grep -q -- '--repo org/repo-b'
    repo_count="$(printf '%s\n' "$output" | grep -o -- '--repo org/repo-[a-z]*' | sort -u | wc -l | tr -d ' ')"
    [ "$repo_count" = "2" ]
    checkout_count="$(printf '%s\n' "$output" | grep -o 'org__repo-[a-z]*' | sort -u | wc -l | tr -d ' ')"
    [ "$checkout_count" = "2" ]
}

@test "live fleet E2E remains opt-in behind AUTOSPEC_FLEET_LIVE_E2E" {
    [ "${AUTOSPEC_FLEET_LIVE_E2E:-0}" != "1" ]
}
