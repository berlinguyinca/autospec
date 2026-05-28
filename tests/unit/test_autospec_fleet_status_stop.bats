#!/usr/bin/env bats
# tests/unit/test_autospec_fleet_status_stop.bats - fleet status and stop.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FLEET_STATUS="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-status.sh"
    FLEET_STOP="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-stop.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-status-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

write_config() {
    cat > "$TEST_TMPDIR/fleet.yml" <<YAML
version: 1
workspace: $TEST_TMPDIR/repos
default_profile: qwen3-32b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-32b-laptop
    enabled: true
  - url: git@github.com:org/repo-b.git
    profile: claude-sonnet-cloud
    enabled: true
YAML
}

write_mock_probe() {
    cat > "$TEST_TMPDIR/list-ready-issues.sh" <<'EOF'
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
  org/repo-a)
    printf '{"ready":[{"number":1}],"blocked":[],"claimed":[],"conflicts":[],"batch":[{"number":1}]}\n'
    ;;
  org/repo-b)
    printf '{"ready":[],"blocked":[{"number":2}],"claimed":[{"number":3}],"conflicts":[],"batch":[]}\n'
    ;;
esac
EOF
    chmod +x "$TEST_TMPDIR/list-ready-issues.sh"
}

write_mock_stop() {
    cat > "$TEST_TMPDIR/autospec-stop.sh" <<EOF
#!/usr/bin/env bash
printf '%s|%s\n' "\$(pwd)" "\$*" >> "$TEST_TMPDIR/stop.log"
EOF
    chmod +x "$TEST_TMPDIR/autospec-stop.sh"
}

@test "fleet-status --json emits repos as a JSON array" {
    write_config
    write_mock_probe

    run bash "$FLEET_STATUS" --json \
        --config "$TEST_TMPDIR/fleet.yml" \
        --list-ready-bin "$TEST_TMPDIR/list-ready-issues.sh"

    [ "$status" -eq 0 ]
    json="$output"
    run bash -c 'printf "%s\n" "$1" | jq -e ".repos | type == \"array\" and length == 2"' _ "$json"
    [ "$status" -eq 0 ]
    run bash -c 'printf "%s\n" "$1" | jq -e ".repos[] | select(.repo == \"org/repo-a\" and .ready == 1 and .batch == 1)"' _ "$json"
    [ "$status" -eq 0 ]
}

@test "fleet-stop --graceful calls autospec-stop once per active repo" {
    write_config
    write_mock_stop
    mkdir -p "$TEST_TMPDIR/repos/org__repo-a"

    run bash "$FLEET_STOP" --graceful \
        --config "$TEST_TMPDIR/fleet.yml" \
        --stop-bin "$TEST_TMPDIR/autospec-stop.sh"

    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/stop.log" ]
    [ "$(wc -l < "$TEST_TMPDIR/stop.log" | tr -d ' ')" = "1" ]
    grep -q "$TEST_TMPDIR/repos/org__repo-a|--graceful" "$TEST_TMPDIR/stop.log"
}
