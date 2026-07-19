#!/usr/bin/env bats
# autospec-run-status.bats — operator status view.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    S="$REPO_ROOT/skills/autospec-run/scripts/autospec-run-status.sh"
    TMP="$(mktemp -d /tmp/autospec-status-XXXXXX)"
    export AUTOSPEC_HEARTBEAT_DIR="$TMP/hb"
    export AUTOSPEC_STATE_DIR="$TMP/state"
    export AUTOSPEC_REPO="me/repo"
    mkdir -p "$TMP/hb/me_repo" "$TMP/state"
    NOW="$(date -u +%s)"
    BASH_BIN="$(command -v bash)"
}
teardown() { rm -rf "$TMP"; }

hb() { # hb <issue> <step> <ts> [pr] [branch]
    printf '{"issue":"%s","branch":"%s","step":"%s","ts":%s,"pr":"%s","repo":"me/repo","host":"h1"}\n' \
        "$1" "${5:-feat/x}" "$2" "$3" "${4:-}" > "$AUTOSPEC_HEARTBEAT_DIR/me_repo/$1.json"
}

status_with_queue_stub() {
    mkdir -p "$TMP/status-bin" "$TMP/status-scripts"
    cp "$S" "$TMP/status-scripts/autospec-run-status.sh"
    chmod +x "$TMP/status-scripts/autospec-run-status.sh"
    cat > "$TMP/status-bin/gh" <<'EOF_GH'
#!/usr/bin/env bash
case "$1 $2" in
  "repo view") printf '{"nameWithOwner":"me/repo"}\n' ;;
  *) printf '[]\n' ;;
esac
EOF_GH
    chmod +x "$TMP/status-bin/gh"
    cat > "$TMP/status-bin/autospec"
    chmod +x "$TMP/status-bin/autospec"
}

@test "shows in-flight issues with step and age" {
    hb 101 tests_passed "$NOW"
    run bash "$S" --repo me/repo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '#101'
    printf '%s\n' "$output" | grep -q 'tests_passed'
}

@test "flags a stale heartbeat (older than --stale-secs)" {
    hb 102 pr_created "$((NOW - 4000))" 55
    run bash "$S" --repo me/repo --stale-secs 1500
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -E '#102' | grep -q 'STALE'
}

@test "fresh heartbeat is not stale" {
    hb 103 implementing "$NOW"
    run bash "$S" --repo me/repo --stale-secs 1500
    printf '%s\n' "$output" | grep -E '#103' | grep -q 'ok'
    ! printf '%s\n' "$output" | grep -E '#103' | grep -q 'STALE'
}

@test "detects the stop flag" {
    hb 104 merged "$NOW"
    touch "$AUTOSPEC_STATE_DIR/stop.flag"
    run bash "$S" --repo me/repo
    printf '%s\n' "$output" | grep -i 'stop-flag' | grep -q 'SET'
}

@test "no heartbeats -> reports none, exit 0" {
    run bash "$S" --repo me/repo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'in-flight: none'
}

@test "--json emits stop_flag + issues + stale boolean" {
    hb 105 pr_created "$((NOW - 4000))" 9
    run bash "$S" --repo me/repo --json --stale-secs 1500
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.issues[0].issue')" = "105" ]
    [ "$(printf '%s' "$output" | jq -r '.issues[0].stale')" = "true" ]
    [ "$(printf '%s' "$output" | jq 'has("queue")')" = "true" ]
}

@test "--json reports claimed issue without heartbeat" {
    status_with_queue_stub <<'EOF_QUEUE'
#!/usr/bin/env bash
cat <<'JSON'
{"ready":[],"blocked":[],"claimed":[{"number":1604,"title":"enumerate running conductors"}],"conflicts":[],"batch":[]}
JSON
EOF_QUEUE

    run env PATH="$TMP/status-bin:$PATH" AUTOSPEC_QUEUE_BIN="$TMP/status-bin/autospec" bash "$TMP/status-scripts/autospec-run-status.sh" --repo me/repo --json

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.claimed_without_heartbeat[0].issue')" = "1604" ]
}

@test "human status prints claimed issue without heartbeat when no heartbeat table exists" {
    status_with_queue_stub <<'EOF_QUEUE'
#!/usr/bin/env bash
cat <<'JSON'
{"ready":[],"blocked":[],"claimed":[{"number":1604,"title":"enumerate running conductors"}],"conflicts":[],"batch":[]}
JSON
EOF_QUEUE

    run env PATH="$TMP/status-bin:$PATH" AUTOSPEC_QUEUE_BIN="$TMP/status-bin/autospec" bash "$TMP/status-scripts/autospec-run-status.sh" --repo me/repo

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'claimed without heartbeat (1):'
    printf '%s\n' "$output" | grep -q '#1604'
    printf '%s\n' "$output" | grep -q 'enumerate running conductors'
}

@test "claimed issue with matching heartbeat is not reported as missing heartbeat" {
    status_with_queue_stub <<'EOF_QUEUE'
#!/usr/bin/env bash
cat <<'JSON'
{"ready":[],"blocked":[],"claimed":[{"number":1604,"title":"enumerate running conductors"}],"conflicts":[],"batch":[]}
JSON
EOF_QUEUE
    mkdir -p "$AUTOSPEC_HEARTBEAT_DIR/me__repo"
    printf '{"issue":1604,"branch":"feat/issue-1604","step":"implementing","ts":%s,"pr":"","repo":"me/repo","host":"h1"}\n' \
        "$NOW" > "$AUTOSPEC_HEARTBEAT_DIR/me__repo/1604.json"

    run env PATH="$TMP/status-bin:$PATH" AUTOSPEC_QUEUE_BIN="$TMP/status-bin/autospec" bash "$TMP/status-scripts/autospec-run-status.sh" --repo me/repo --json

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.claimed_without_heartbeat | length')" = "0" ]
}

@test "jq missing -> exit 2 (fail-closed)" {
    mkdir -p "$TMP/empty"
    run env PATH="$TMP/empty" "$BASH_BIN" "$S" --repo me/repo
    [ "$status" -eq 2 ]
}
