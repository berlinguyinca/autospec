#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t autospec-verify-bound.XXXXXX)"
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
while :; do
    printf 'chatty harness heartbeat\n'
    sleep 0.2
done
EOF
    chmod +x "$TMP/bin/omx"
    export PATH="$TMP/bin:$PATH"
    printf '{"deduped":[{"norm_title":"fix parser","title":"fix parser","evidence":"README.md:1","estimated_complexity":"small"}]}' > "$TMP/dedup.json"
    export AUTOSPEC_EXPLORE_DEDUPED_IN="$TMP/dedup.json"
    export AUTOSPEC_EXPLORE_VERDICTS_OUT="$TMP/verdicts.json"
    export AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS=1
    export AUTOSPEC_AUTONOMOUS_VERIFY_POLL_SECS=1
    export AUTOSPEC_AUTONOMOUS_VERIFY_MAX_SECS=3
}

teardown() { rm -rf "$TMP"; }

@test "chatty verifier is terminated by absolute runtime bound" {
    run bash "$REPO_ROOT/scripts/autospec-autonomous-verify-drain.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"absolute timeout after 3s"* ]]
    [ -s "$AUTOSPEC_EXPLORE_VERDICTS_OUT" ]
}
