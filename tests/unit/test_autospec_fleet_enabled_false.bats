#!/usr/bin/env bats
# tests/unit/test_autospec_fleet_enabled_false.bats
#
# Regression coverage for the yq `//` alternative-operator bug: `//` treats
# a literal `false` as absent (same as jq), so `enabled: false // true`
# silently read back as `true` and an operator's explicit disable was
# ignored. This proves fleet-run.sh honors an explicit `enabled: false` by
# never spawning a conductor for that repo, that an absent `enabled` key
# still defaults to enabled, and that fleet-status.sh / fleet-stop.sh honor
# the same explicit disable.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FLEET_RUN="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-run.sh"
    FLEET_STATUS="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-status.sh"
    FLEET_STOP="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-stop.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-fleet-enabled-false-XXXXXX)"
    export AUTOSPEC_HEARTBEAT_DIR="$TEST_TMPDIR/heartbeats"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
    unset AUTOSPEC_HEARTBEAT_DIR
    unset AUTOSPEC_FLEET_AUTONOMOUS_BIN
}

# repo-a is explicitly disabled; repo-b has no `enabled` key at all (must
# still default to enabled, per the deliberate "absent means true" rule).
write_fleet_config() {
    cat > "$TEST_TMPDIR/fleet.yml" <<YAML
version: 1
workspace: $TEST_TMPDIR/repos
default_profile: qwen3-6-35b-a3b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: false
  - url: git@github.com:org/repo-b.git
    profile: qwen3-6-35b-a3b-laptop
YAML
}

write_mock_queue() {
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
  org/repo-a|org/repo-b)
    printf '{"ready":[{"number":1}],"blocked":[],"claimed":[],"conflicts":[],"batch":[{"number":1}]}\n'
    ;;
  *)
    printf '{"ready":[],"blocked":[],"claimed":[],"conflicts":[],"batch":[]}\n'
    ;;
esac
EOF
    chmod +x "$TEST_TMPDIR/autospec"
}

# A stub that MUST shadow the real `autospec-autonomous` binary: it logs its
# full argv to a file and exits 0, so a call proves a spawn attempt without
# ever reaching a real conductor. A test asserting "no spawn" checks this
# log file is absent/empty, not printed text — printed text alone would not
# catch a real accidental spawn.
write_stub_autonomous() {
    cat > "$TEST_TMPDIR/autospec-autonomous" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TEST_TMPDIR/autonomous.log"
exit 0
EOF
    chmod +x "$TEST_TMPDIR/autospec-autonomous"
    export AUTOSPEC_FLEET_AUTONOMOUS_BIN="$TEST_TMPDIR/autospec-autonomous"
}

@test "fleet-run does not spawn a conductor for a repo with enabled: false" {
    write_fleet_config
    write_mock_queue
    write_stub_autonomous
    mkdir -p "$TEST_TMPDIR/repos/org__repo-a" "$TEST_TMPDIR/repos/org__repo-b"

    run bash "$FLEET_RUN" --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    # The stub must never have been invoked for repo-a.
    if [ -f "$TEST_TMPDIR/autonomous.log" ]; then
        run grep -q 'repo-a' "$TEST_TMPDIR/autonomous.log"
        [ "$status" -ne 0 ]
    fi
}

@test "fleet-run spawns a conductor for a repo with no enabled key (defaults true)" {
    write_fleet_config
    write_mock_queue
    write_stub_autonomous
    mkdir -p "$TEST_TMPDIR/repos/org__repo-a" "$TEST_TMPDIR/repos/org__repo-b"

    run bash "$FLEET_RUN" --once \
        --config "$TEST_TMPDIR/fleet.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/autonomous.log" ]
    run grep -q -- '--repo org/repo-b' "$TEST_TMPDIR/autonomous.log"
    [ "$status" -eq 0 ]
}

@test "fleet-status reports a repo with enabled: false without probing its queue" {
    write_fleet_config
    write_mock_queue

    run bash "$FLEET_STATUS" --json \
        --config "$TEST_TMPDIR/fleet.yml" \
        --queue-bin "$TEST_TMPDIR/autospec"

    [ "$status" -eq 0 ]
    json="$output"
    run bash -c 'printf "%s\n" "$1" | jq -e ".repos[] | select(.repo == \"org/repo-a\") | .ready == 0 and .batch == 0"' _ "$json"
    [ "$status" -eq 0 ]
}

@test "fleet-stop does not call autospec-stop for a repo with enabled: false" {
    write_fleet_config
    cat > "$TEST_TMPDIR/autospec-stop.sh" <<EOF
#!/usr/bin/env bash
printf '%s|%s\n' "\$(pwd)" "\$*" >> "$TEST_TMPDIR/stop.log"
EOF
    chmod +x "$TEST_TMPDIR/autospec-stop.sh"
    mkdir -p "$TEST_TMPDIR/repos/org__repo-a" "$TEST_TMPDIR/repos/org__repo-b"

    run bash "$FLEET_STOP" --graceful \
        --config "$TEST_TMPDIR/fleet.yml" \
        --stop-bin "$TEST_TMPDIR/autospec-stop.sh"

    [ "$status" -eq 0 ]
    if [ -f "$TEST_TMPDIR/stop.log" ]; then
        run grep -q 'repo-a' "$TEST_TMPDIR/stop.log"
        [ "$status" -ne 0 ]
        # repo-b (default enabled) must have been stopped instead.
        run grep -q "org__repo-b" "$TEST_TMPDIR/stop.log"
        [ "$status" -eq 0 ]
    else
        false
    fi
}
