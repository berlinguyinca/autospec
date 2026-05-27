#!/usr/bin/env bats
# tests/unit/test_autospec_watchdog_run_state.bats — GitHub run-state reclaim.

WATCHDOG="${BATS_TEST_DIRNAME}/../../scripts/autospec-watchdog.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="testorg/testrepo"
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=300
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=10800
    export AUTOSPEC_WATCHDOG_STALE_SECS=1800
    export LABELS="$TEST_TMP/labels.txt"
    export COMMENTS="$TEST_TMP/comments.json"
    export PR_STATE="$TEST_TMP/pr-state.txt"
    export CALLS="$TEST_TMP/calls.log"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR"
    printf 'OPEN in-progress-by-bot\n' > "$LABELS"
    printf 'OPEN\n' > "$PR_STATE"

    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "${CALLS:?}"

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'testorg/testrepo\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  jq -r '.[].number' "${COMMENTS:?}"
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  if printf '%s\n' "$*" | grep -q -- '--json comments'; then
    issue="$3"
    jq --argjson issue "$issue" \
      -r '[.[] | select(.number == $issue) | .body][0] // ""' "${COMMENTS:?}"
  else
    cat "${LABELS:?}"
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  remove=""
  add=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --remove-label) remove="$2"; shift 2 ;;
      --add-label) add="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [ "$remove" = "in-progress-by-bot" ] && [ "$add" = "auto-implement" ]; then
    printf 'OPEN auto-implement\n' > "$LABELS"
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  cat "$PR_STATE"
  exit 0
fi

exit 0
SH
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TEST_TMP"
}

write_state_comment() {
    local issue="$1"
    local state="$2"
    local updated_at="$3"
    local pr="$4"
    jq -n \
      --argjson issue "$issue" \
      --arg state "$state" \
      --arg updated_at "$updated_at" \
      --arg pr "$pr" \
      '[{
        number: $issue,
        body: ("<!-- autospec-run-state:begin -->\n" +
          ({schema:1,repo:"testorg/testrepo",issue:$issue,worker_id:"worker-a",state:$state,branch:"feat/x",pr:$pr,step:$state,paths:[],claimed_at:$updated_at,updated_at:$updated_at,ttl_seconds:300} | tojson) +
          "\n<!-- autospec-run-state:end -->")
      }]'
}

@test "watchdog restores auto-implement for stale claimed state" {
    stale="$(date -u -v-10M +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '10 minutes ago' +'%Y-%m-%dT%H:%M:%SZ')"
    write_state_comment 42 claimed "$stale" "" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

@test "watchdog keeps pr_created state with open PR 7" {
    stale="$(date -u -v-4H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '4 hours ago' +'%Y-%m-%dT%H:%M:%SZ')"
    write_state_comment 42 pr_created "$stale" "7" > "$COMMENTS"
    printf 'OPEN\n' > "$PR_STATE"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=0"* ]]
    grep -q 'in-progress-by-bot' "$LABELS"
}
