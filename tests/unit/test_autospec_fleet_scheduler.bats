#!/usr/bin/env bats
# tests/unit/test_autospec_fleet_scheduler.bats - fleet dry-run scheduler.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FLEET_RUN="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-run.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-scheduler-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

write_mock_probe() {
    cat > "$TEST_TMPDIR/autospec" <<'EOF'
#!/usr/bin/env bash
repo=""
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="$2"; shift 2 ;;
    --batch-size) shift 2 ;;
    *) shift ;;
  esac
done
case "$repo" in
  org/repo-a|org/repo-b|org/repo-c)
    printf '{"batch":[{"number":1}]}\n'
    ;;
  *)
    printf '{"batch":[]}\n'
    ;;
esac
EOF
    chmod +x "$TEST_TMPDIR/autospec"
}

write_fleet_config() {
    cat > "$TEST_TMPDIR/fleet.yml" <<'YAML'
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-6-35b-a3b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
  - url: git@github.com:org/repo-b.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
  - url: https://github.com/org/repo-c.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
YAML
}

write_node_config() {
    cat > "$TEST_TMPDIR/fleet-node.yml" <<'YAML'
node_id: test-node
workspace: /tmp/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-6-35b-a3b-laptop
YAML
}

@test "fleet-run dry-run emits autospec-run commands for eligible repos" {
    write_mock_probe
    write_fleet_config
    write_node_config

    run bash "$FLEET_RUN" --dry-run --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --node-config "$TEST_TMPDIR/fleet-node.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q -- 'autospec-autonomous start --detach'
    printf '%s\n' "$output" | grep -q -- '--repo org/repo-a'
    printf '%s\n' "$output" | grep -q -- '--repo org/repo-b'
    printf '%s\n' "$output" | grep -q -- 'org__repo-a'
    printf '%s\n' "$output" | grep -q -- 'org__repo-b'
}

@test "fleet-run caps output at parallel_repos 2" {
    write_mock_probe
    write_fleet_config
    write_node_config

    run bash "$FLEET_RUN" --dry-run --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --node-config "$TEST_TMPDIR/fleet-node.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    count="$(printf '%s\n' "$output" | grep -c -- 'autospec-autonomous start --detach')"
    [ "$count" -eq 2 ]
    if printf '%s\n' "$output" | grep -q -- 'org/repo-c'; then
        false
    fi
}

@test "fleet-run skips repos with profiles unavailable on the node" {
    write_mock_probe
    write_fleet_config
    cat > "$TEST_TMPDIR/fleet-node.yml" <<'YAML'
node_id: test-node
max_parallel_repos: 2
profiles:
  - claude-sonnet-cloud
YAML

    run bash "$FLEET_RUN" --dry-run --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --node-config "$TEST_TMPDIR/fleet-node.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    [ -z "$output" ]
}
