#!/usr/bin/env bats
# tests/refresh-queue.bats — TDD for scripts/refresh-queue.sh (issue #3655).
#
# The dispatch queue is regenerated from the live tracker (open + auto-implement)
# on every refresh: surviving entries keep their previous position, newly
# dispatchable ones are appended in tracker order, resolved entries are pruned,
# and the run reports the delta ("202 -> 134, dropped 86 resolved, added 18 newly
# labelled"). The #4033 PR-coverage exclusion still applies as a sub-filter.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/refresh-queue.sh"

setup() {
    WORK="$(mktemp -d -t refresh-queue-test.XXXXXX)"
    BIN="$WORK/bin"
    mkdir -p "$BIN"
    # Mock gh: serve repo / pulls / issues from env-var-pointed files so each test
    # can pin the tracker and PR state without touching the network.
    cat > "$BIN/gh" <<'MOCK'
#!/usr/bin/env bash
if [ "$1" = "api" ]; then
  case "$2" in
    repo) echo "${MOCK_REPO_JSON:-{\"full_name\":\"me/repo\"}}"; exit 0 ;;
    *pulls*) cat "${MOCK_PRS_FILE}"; exit 0 ;;
    *issues*) cat "${MOCK_ISSUES_FILE}"; exit 0 ;;
  esac
fi
echo "mock gh: unexpected: $*" >&2
exit 1
MOCK
    chmod +x "$BIN/gh"
    export PATH="$BIN:$PATH"
    export MOCK_ISSUES_FILE="$WORK/issues.json"
    export MOCK_PRS_FILE="$WORK/prs.json"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# write_issues <json-array> — pin the open + auto-implement tracker (fetch order).
write_issues() { printf '%s\n' "$1" > "$MOCK_ISSUES_FILE"; }
# write_prs <json-array> — pin the open PRs.
write_prs() { printf '%s\n' "$1" > "$MOCK_PRS_FILE"; }
# run_refresh [extra args] — run the script against the pinned tracker into $WORK/queue.json.
run_refresh() {
    run bash "$SCRIPT" --repo me/repo --out "$WORK/queue.json" "$@"
}
# assert_queue <json-array> — the written queue file equals <json-array>.
assert_queue() {
    run jq -c '.' "$WORK/queue.json"
    [ "$status" -eq 0 ]
    [ "$output" = "$1" ]
}

@test "refresh-queue.sh: bash -n syntax check" {
    run bash -n "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "refresh-queue.sh: --help exits 0 and describes regeneration" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"regenerate the Phase 4 dispatch queue"* ]]
    [[ "$output" == *"open + auto-implement"* ]]
}

@test "refresh-queue.sh: preserves the previous order of live entries, appends new ones" {
    # Tracker in fetch order 300,100,600,200: 300/100/200 still live, 600 newly
    # labelled, 500/400 no longer open. Previous queue is deliberately unsorted so a
    # re-sort would be detectable.
    write_prs '[]'
    write_issues '[{"number":300,"pull_request":null},{"number":100,"pull_request":null},{"number":600,"pull_request":null},{"number":200,"pull_request":null}]'
    printf '[500,100,300,200,400]' > "$WORK/queue.json"

    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"queue: 5 -> 4, dropped 2 resolved, added 1 newly labelled"* ]]
    [[ "$output" == *"excluded 0 issues that already have an open PR"* ]]
    # Survivors keep their PREVIOUS positions (100,300,200); 600 is appended. A
    # re-sort would yield [100,200,300,600] instead.
    assert_queue '[100,300,200,600]'
}

@test "refresh-queue.sh: first run has an empty previous queue (all added, none dropped)" {
    write_prs '[]'
    # Fetch order 300,100,200 (unsorted) must be preserved on a first run.
    write_issues '[{"number":300,"pull_request":null},{"number":100,"pull_request":null},{"number":200,"pull_request":null}]'
    # No pre-seeded queue.json — a first run.

    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"queue: 0 -> 3, dropped 0 resolved, added 3 newly labelled"* ]]
    assert_queue '[300,100,200]'
}

@test "refresh-queue.sh: excludes PR-covered issues and prunes them from the queue" {
    # 200 is covered by head branch fix/issue-200; 100 by "Closes #100" in a body.
    write_prs '[{"number":900,"head":{"ref":"fix/issue-200"},"body":""},{"number":901,"head":{"ref":"feat/x"},"body":"Closes #100"}]'
    write_issues '[{"number":100,"pull_request":null},{"number":200,"pull_request":null},{"number":300,"pull_request":null}]'
    printf '[100,200,300,500]' > "$WORK/queue.json"

    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"excluded 2 issues that already have an open PR"* ]]
    [[ "$output" == *"queue: 4 -> 1, dropped 3 resolved, added 0 newly labelled"* ]]
    assert_queue '[300]'
}

@test "refresh-queue.sh: a resolved issue with no PR is pruned (dropped, not excluded)" {
    write_prs '[]'
    # Tracker only has 100; 200 was resolved/closed since the last refresh.
    write_issues '[{"number":100,"pull_request":null}]'
    printf '[100,200]' > "$WORK/queue.json"

    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"excluded 0 issues that already have an open PR"* ]]
    [[ "$output" == *"queue: 2 -> 1, dropped 1 resolved, added 0 newly labelled"* ]]
    assert_queue '[100]'
}

@test "refresh-queue.sh: a PR closed unmerged makes its issue reappear on the next refresh" {
    # Issue 200 is in the previous queue but currently PR-covered, so it is dropped.
    write_prs '[{"number":900,"head":{"ref":"fix/issue-200"},"body":""}]'
    write_issues '[{"number":200,"pull_request":null}]'
    printf '[200]' > "$WORK/queue.json"

    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"excluded 1 issues that already have an open PR"* ]]
    assert_queue '[]'

    # Now the PR is closed unmerged: no open PRs, 200 is dispatchable again.
    write_prs '[]'
    run_refresh
    [ "$status" -eq 0 ]
    [[ "$output" == *"excluded 0 issues that already have an open PR"* ]]
    [[ "$output" == *"queue: 0 -> 1, dropped 0 resolved, added 1 newly labelled"* ]]
    assert_queue '[200]'
}

@test "refresh-queue.sh: --check refuses an issue that already has an open PR (exit 2)" {
    write_prs '[{"number":900,"head":{"ref":"fix/issue-200"},"body":""}]'
    write_issues '[]'

    run_refresh --check 200
    [ "$status" -eq 2 ]
    [[ "$output" == *"already has an open PR"* ]]
}

@test "refresh-queue.sh: --check passes an issue without an open PR (exit 0)" {
    write_prs '[{"number":900,"head":{"ref":"fix/issue-200"},"body":""}]'
    write_issues '[]'

    run_refresh --check 300
    [ "$status" -eq 0 ]
}

@test "refresh-queue.sh: queue file is written private (0600)" {
    write_prs '[]'
    write_issues '[{"number":100,"pull_request":null}]'

    run_refresh
    [ "$status" -eq 0 ]
    run stat -c '%a' "$WORK/queue.json"
    [ "$status" -eq 0 ]
    [ "$output" = "600" ]
}
