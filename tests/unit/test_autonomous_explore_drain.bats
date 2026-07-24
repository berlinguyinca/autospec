#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t autospec-explore-bound.XXXXXX)"
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
while :; do
    printf 'chatty explore heartbeat\n'
    sleep 0.2
done
EOF
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
    *'issue list'*) printf '[]\n' ;;
    *) printf 'test/repo\n' ;;
esac
EOF
    chmod +x "$TMP/bin/omx" "$TMP/bin/gh"
    export PATH="$TMP/bin:$PATH"
    export CONDUCTOR_REPO=test/repo
    export AUTOSPEC_REPO_DIR="$TMP"
    export AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS=1
    export AUTOSPEC_AUTONOMOUS_EXPLORE_POLL_SECS=1
    export AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS=3
}

teardown() { rm -rf "$TMP"; }

@test "chatty explore harness is terminated by absolute runtime bound" {
    run bash "$REPO_ROOT/scripts/autospec-autonomous-explore-drain.sh" --once
    [ "$status" -eq 0 ]
    [[ "$output" == *'max runtime 3s reached'* ]]
    [[ "$output" == *'"dry":true'* ]]
}
