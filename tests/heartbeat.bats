#!/usr/bin/env bats
# tests/heartbeat.bats — tests for heartbeat-write.sh and heartbeat-read.sh
# Covers: repo-scoped layout, collision-isolation, flat-format migration via watchdog.

HB_WRITE="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/heartbeat-write.sh"
HB_READ="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/heartbeat-read.sh"
RUN_STATUS="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/autospec-run-status.sh"
WATCHDOG="${BATS_TEST_DIRNAME}/../scripts/autospec-watchdog.sh"

setup() {
    # Create isolated temp dir for each test
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_HEARTBEAT_DIR="$TEST_TMP"
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP"

    # Create a stub gh command that returns a predictable repo
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
# Stub gh for testing
if [[ "$*" == *"repo view"* ]]; then
    echo "testorg/testrepo"
    exit 0
fi
if [[ "$*" == *"issue view"* ]]; then
    echo "OPEN auto-implement,in-progress-by-bot"
    exit 0
fi
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── heartbeat-write.sh ────────────────────────────────────────────────────────

@test "heartbeat-write.sh is executable" {
    [ -x "$HB_WRITE" ]
}

@test "heartbeat-write.sh --help exits 0" {
    run bash "$HB_WRITE" --help
    [ "$status" -eq 0 ]
}

@test "heartbeat-write.sh creates file under repo-slug subdir" {
    run bash "$HB_WRITE" --issue 42 --step claimed --repo "testorg/testrepo"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/testorg__testrepo/42.json" ]
}

@test "heartbeat-write.sh file contains correct JSON fields" {
    bash "$HB_WRITE" --issue 99 --step pr_created --branch feat/test --pr 123 --repo "myorg/myrepo"
    local content
    content="$(cat "$TEST_TMP/myorg__myrepo/99.json")"
    echo "$content" | grep -q '"issue":"99"'
    echo "$content" | grep -q '"step":"pr_created"'
    echo "$content" | grep -q '"branch":"feat/test"'
    echo "$content" | grep -q '"pr":"123"'
    echo "$content" | grep -q '"repo":"myorg/myrepo"'
}

@test "heartbeat-write.sh uses AUTOSPEC_REPO when --repo not given" {
    export AUTOSPEC_REPO="envorg/envrepo"
    bash "$HB_WRITE" --issue 77 --step claimed
    [ -f "$TEST_TMP/envorg__envrepo/77.json" ]
}

@test "heartbeat session binding preserves the failed target after an issue successor" {
    bash "$HB_WRITE" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-old --session-id session-old
    bash "$HB_WRITE" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-new --session-id session-new

    run bash "$HB_READ" --session-id session-old --repo testorg/testrepo
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"session_id":"session-old"'
    echo "$output" | grep -q '"claim_id":"claim-generation-old"'
    echo "$output" | grep -q '"worker_id":"worker-a"'

    current="$(bash "$HB_READ" --issue 42 --repo testorg/testrepo)"
    echo "$current" | grep -q '"session_id":"session-new"'
    echo "$current" | grep -q '"claim_id":"claim-generation-new"'
}

@test "heartbeat session lookup fails closed for a legacy heartbeat" {
    bash "$HB_WRITE" --issue 42 --step claimed --repo testorg/testrepo

    run bash "$HB_READ" --session-id session-old --repo testorg/testrepo
    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'no durable heartbeat binding'
}

@test "heartbeat session lookup reads the Rust collision-safe repository directory" {
    session_key="$(printf '%s' session-rust | od -An -tx1 | tr -d ' \n')"
    mkdir -p "$TEST_TMP/o7_testorg_r8_testrepo/sessions"
    printf '{"issue":"42","branch":"feat/test","step":"claimed","ts":1,"pr":"","repo":"testorg/testrepo","worker_id":"worker-a","claim_id":"claim-rust","session_id":"session-rust"}\n' \
        > "$TEST_TMP/o7_testorg_r8_testrepo/sessions/${session_key}.json"

    run bash "$HB_READ" --session-id session-rust --repo testorg/testrepo
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"claim_id":"claim-rust"'
}

@test "heartbeat writer rejects a partial session binding" {
    run bash "$HB_WRITE" --issue 42 --step claimed --repo testorg/testrepo \
        --session-id session-old --claim-id claim-generation-old
    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'session binding requires --session-id, --claim-id, and --worker-id'
}

@test "heartbeat session binding rejects a different generation and preserves the original" {
    bash "$HB_WRITE" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-old --session-id session-shared
    session_key="$(printf '%s' session-shared | od -An -tx1 | tr -d ' \n')"
    binding="$TEST_TMP/testorg__testrepo/sessions/${session_key}.json"
    original="$(cat "$binding")"

    run bash "$HB_WRITE" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-new --session-id session-shared

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'session binding identity conflict'
    [ "$(cat "$binding")" = "$original" ]
    grep -q '"claim_id":"claim-generation-old"' "$TEST_TMP/testorg__testrepo/42.json"
}

@test "heartbeat session binding accepts an identical identity refresh without replacing the sidecar" {
    bash "$HB_WRITE" --issue 42 --step claimed --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-old --session-id session-shared
    session_key="$(printf '%s' session-shared | od -An -tx1 | tr -d ' \n')"
    binding="$TEST_TMP/testorg__testrepo/sessions/${session_key}.json"
    original="$(cat "$binding")"

    run bash "$HB_WRITE" --issue 42 --step tests_started --branch feat/test --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-generation-old --session-id session-shared

    [ "$status" -eq 0 ]
    [ "$(cat "$binding")" = "$original" ]
    grep -q '"step":"tests_started"' "$TEST_TMP/testorg__testrepo/42.json"
}

@test "heartbeat session binding fills in a branch that was unknown at the first write" {
    bash "$HB_WRITE" --issue 42 --step expand_start --branch "" --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-late-branch
    session_key="$(printf '%s' session-late-branch | od -An -tx1 | tr -d ' \n')"
    binding="$TEST_TMP/testorg__testrepo/sessions/${session_key}.json"
    grep -q '"branch":""' "$binding"

    run bash "$HB_WRITE" --issue 42 --step worktree_ready --branch feat/late --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-late-branch

    [ "$status" -eq 0 ]
    grep -q '"branch":"feat/late"' "$binding"
    grep -q '"branch":"feat/late"' "$TEST_TMP/testorg__testrepo/42.json"
    read_back="$(bash "$HB_READ" --session-id session-late-branch --repo testorg/testrepo)"
    echo "$read_back" | jq -e '.branch == "feat/late"' >/dev/null
}

@test "a branch filled in once is not re-writable by a third branch" {
    bash "$HB_WRITE" --issue 42 --step expand_start --branch "" --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-branch-once
    bash "$HB_WRITE" --issue 42 --step worktree_ready --branch feat/late --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-branch-once
    session_key="$(printf '%s' session-branch-once | od -An -tx1 | tr -d ' \n')"
    binding="$TEST_TMP/testorg__testrepo/sessions/${session_key}.json"
    filled="$(cat "$binding")"

    run bash "$HB_WRITE" --issue 42 --step tests_started --branch feat/other --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-branch-once

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'session binding identity conflict'
    [ "$(cat "$binding")" = "$filled" ]
}

@test "an unknown branch does not let a different claim adopt the session" {
    bash "$HB_WRITE" --issue 42 --step expand_start --branch "" --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-branch-guard
    session_key="$(printf '%s' session-branch-guard | od -An -tx1 | tr -d ' \n')"
    binding="$TEST_TMP/testorg__testrepo/sessions/${session_key}.json"
    original="$(cat "$binding")"

    run bash "$HB_WRITE" --issue 43 --step worktree_ready --branch feat/other --repo testorg/testrepo \
        --worker-id worker-b --claim-id claim-b --session-id session-branch-guard

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'session binding identity conflict'
    [ "$(cat "$binding")" = "$original" ]
}

@test "concurrent heartbeat writers cannot overwrite one session identity" {
    set +e
    (bash "$HB_WRITE" --issue 42 --step claimed --branch feat/a --repo testorg/testrepo \
        --worker-id worker-a --claim-id claim-a --session-id session-race; printf '%s' "$?" > "$TEST_TMP/a.rc") &
    a_pid=$!
    (bash "$HB_WRITE" --issue 43 --step claimed --branch feat/b --repo testorg/testrepo \
        --worker-id worker-b --claim-id claim-b --session-id session-race; printf '%s' "$?" > "$TEST_TMP/b.rc") &
    b_pid=$!
    wait "$a_pid"
    wait "$b_pid"
    set -e

    a_rc="$(cat "$TEST_TMP/a.rc")"
    b_rc="$(cat "$TEST_TMP/b.rc")"
    [ $((a_rc + b_rc)) -ne 0 ]
    [ $((a_rc * b_rc)) -eq 0 ]
    binding="$(bash "$HB_READ" --session-id session-race --repo testorg/testrepo)"
    echo "$binding" | jq -e '(.claim_id == "claim-a" and .issue == "42") or (.claim_id == "claim-b" and .issue == "43")' >/dev/null
}

@test "heartbeat writer rejects non-canonical issue values without path traversal" {
    for issue in '../escape' '0' '-1' 'abc' '01'; do
        run bash "$HB_WRITE" --issue "$issue" --step claimed --repo testorg/testrepo
        [ "$status" -ne 0 ]
        echo "$output" | grep -q -- '--issue must be a canonical positive integer'
    done
    [ ! -e "$TEST_TMP/escape.json" ]
    [ ! -e "$TEST_TMP/testorg__testrepo/0.json" ]
}

@test "heartbeat reader rejects non-canonical issue values without reading outside the repo directory" {
    mkdir -p "$TEST_TMP/testorg__testrepo"
    printf 'outside-secret\n' > "$TEST_TMP/escape.json"
    for issue in '../escape' '0' '-1' 'abc' '01'; do
        run bash "$HB_READ" --issue "$issue" --repo testorg/testrepo
        [ "$status" -ne 0 ]
        echo "$output" | grep -q -- '--issue must be a canonical positive integer'
        ! echo "$output" | grep -q 'outside-secret'
    done
}

# ── heartbeat-read.sh ─────────────────────────────────────────────────────────

@test "heartbeat-read.sh is executable" {
    [ -x "$HB_READ" ]
}

@test "heartbeat-read.sh --help exits 0" {
    run bash "$HB_READ" --help
    [ "$status" -eq 0 ]
}

@test "heartbeat-read.sh --issue returns correct file content" {
    bash "$HB_WRITE" --issue 55 --step worktree_ready --repo "myorg/myrepo"
    run bash "$HB_READ" --issue 55 --repo "myorg/myrepo"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"issue":"55"'
}

@test "heartbeat-read.sh --issue returns empty when not found" {
    run bash "$HB_READ" --issue 999 --repo "myorg/myrepo"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "heartbeat-read.sh lists only own repo's heartbeats (collision-isolation)" {
    # Write heartbeats for two different repos with same issue number
    bash "$HB_WRITE" --issue 100 --step claimed --repo "repoA/proj"
    bash "$HB_WRITE" --issue 100 --step claimed --repo "repoB/proj"

    # repoA reader should only see repoA's heartbeat
    run bash "$HB_READ" --repo "repoA/proj"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "repoA__proj"
    # Should NOT see repoB's path
    ! echo "$output" | grep -q "repoB__proj"
}

@test "heartbeat-read.sh: zero cross-repo bleed for same issue number" {
    # repoA has issue 50; repoB also has issue 50
    bash "$HB_WRITE" --issue 50 --step merged --repo "orgA/repoX"
    bash "$HB_WRITE" --issue 50 --step claimed --repo "orgB/repoY"

    # Reading issue 50 for repoA returns only repoA's heartbeat
    content="$(bash "$HB_READ" --issue 50 --repo "orgA/repoX")"
    echo "$content" | grep -q '"repo":"orgA/repoX"'
    # Reading issue 50 for repoB returns only repoB's heartbeat
    content="$(bash "$HB_READ" --issue 50 --repo "orgB/repoY")"
    echo "$content" | grep -q '"repo":"orgB/repoY"'
}

@test "heartbeat-read.sh --issue prefers newer legacy heartbeat over older canonical heartbeat" {
    mkdir -p "$TEST_TMP/testorg__testrepo" "$TEST_TMP/testorg-testrepo"
    printf '{"issue":"42","branch":"feat/x","step":"claimed","ts":100,"pr":"","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg__testrepo/42.json"
    printf '{"issue":"42","branch":"feat/x","step":"tests_started","updated_at":"2026-07-11T07:36:25Z","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg-testrepo/42.json"
    touch -t 202607110700 "$TEST_TMP/testorg__testrepo/42.json"
    touch -t 202607110736 "$TEST_TMP/testorg-testrepo/42.json"

    run bash "$HB_READ" --issue 42 --repo "testorg/testrepo"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"step":"tests_started"'
}

@test "autospec-run-status prefers newer legacy heartbeat over older canonical heartbeat" {
    mkdir -p "$TEST_TMP/testorg__testrepo" "$TEST_TMP/testorg-testrepo"
    printf '{"issue":"42","branch":"feat/x","step":"claimed","ts":100,"pr":"","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg__testrepo/42.json"
    printf '{"issue":"42","branch":"feat/x","step":"tests_started","updated_at":"2026-07-11T07:36:25Z","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg-testrepo/42.json"
    touch -t 202607110700 "$TEST_TMP/testorg__testrepo/42.json"
    touch -t 202607110736 "$TEST_TMP/testorg-testrepo/42.json"

    run bash "$RUN_STATUS" --repo "testorg/testrepo" --json
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"step":"tests_started"'
    ! echo "$output" | grep -q '"step":"claimed"'
}

@test "autospec-run-status hides unclaimed stale heartbeats when queue state is known" {
    mkdir -p "$TEST_TMP/status-bin" "$TEST_TMP/testorg__testrepo"
    cp "$RUN_STATUS" "$TEST_TMP/status-bin/autospec-run-status.sh"
    cat > "$TEST_TMP/status-bin/autospec" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"ready":[],"blocked":[],"claimed":[{"number":42,"title":"active"}],"conflicts":[],"batch":[]}
JSON
EOF
    chmod +x "$TEST_TMP/status-bin/autospec-run-status.sh" "$TEST_TMP/status-bin/autospec"
    printf '{"issue":"42","branch":"feat/x","step":"tests_started","ts":100,"pr":"","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg__testrepo/42.json"
    printf '{"issue":"99","branch":"feat/old","step":"pr_created","ts":100,"pr":"101","repo":"testorg/testrepo"}\n' \
        > "$TEST_TMP/testorg__testrepo/99.json"

    run env AUTOSPEC_QUEUE_BIN="$TEST_TMP/status-bin/autospec" bash "$TEST_TMP/status-bin/autospec-run-status.sh" --repo "testorg/testrepo" --json
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"issue":42'
    ! echo "$output" | grep -q '"issue":99'
}

# ── watchdog migration ────────────────────────────────────────────────────────

@test "watchdog migrates flat-format heartbeat with repo field to correct subdir" {
    # Create a flat-format heartbeat (old layout)
    local ts
    ts="$(date -u +%s)"
    printf '{"issue":"200","branch":"feat/x","step":"pr_created","ts":%s,"pr":"50","repo":"migorg/migrepo"}\n' \
        "$ts" > "$TEST_TMP/200.json"

    # Run watchdog migration (skipping gh calls by using stub)
    export AUTOSPEC_WATCHDOG_REPO="migorg/migrepo"
    export AUTOSPEC_WATCHDOG_STALE_SECS=9999
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=99999
    # Run just the migration portion — watchdog exits 0 even without jq for the reconciliation loop
    bash "$WATCHDOG" 2>/dev/null || true

    # The flat file should be gone and replaced in subdir
    [ ! -f "$TEST_TMP/200.json" ]
    [ -f "$TEST_TMP/migorg__migrepo/200.json" ]
}

@test "watchdog deletes stale flat-format heartbeat without repo field" {
    # Create a flat-format heartbeat older than 1 hour
    local stale_ts=$(( $(date -u +%s) - 7200 ))
    printf '{"issue":"300","branch":"","step":"claimed","ts":%s,"pr":"","repo":""}\n' \
        "$stale_ts" > "$TEST_TMP/300.json"

    export AUTOSPEC_WATCHDOG_REPO="someorg/somerepo"
    bash "$WATCHDOG" 2>/dev/null || true

    # Stale flat file without repo field should be deleted
    [ ! -f "$TEST_TMP/300.json" ]
}
