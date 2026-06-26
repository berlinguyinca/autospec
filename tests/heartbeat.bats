#!/usr/bin/env bats
# tests/heartbeat.bats — tests for heartbeat-write.sh and heartbeat-read.sh
# Covers: repo-scoped layout, collision-isolation, flat-format migration via watchdog.

HB_WRITE="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/heartbeat-write.sh"
HB_READ="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/heartbeat-read.sh"
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
