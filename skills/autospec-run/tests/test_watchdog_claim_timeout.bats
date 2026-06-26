#!/usr/bin/env bats
# skills/autospec-run/tests/test_watchdog_claim_timeout.bats
#
# T1–T5: claimed-timeout + GitHub authority cross-check (issue #1346).
#
# These tests drive scripts/autospec-watchdog.sh via a PATH-stubbed gh shim and
# a fixture heartbeat directory.  They cover ONLY the local-heartbeat-path
# reclaim decision at the claimed-timeout gate — the reconcile pass is isolated
# (ISSUE_LIST points to an empty file) so each test exercises exactly one
# scenario.
#
# Tests:
#   T1 — age < 1800s → claimed heartbeat NOT released.
#   T2 — age ≥ 1800s but GitHub run-state comment is fresh → NOT released.
#   T3 — age ≥ 1800s with stale/absent run-state comment → released.
#   T4 — gh exits non-zero during run-state cross-check → NOT released (fail-safe).
#   T5 — default WATCHDOG_CLAIMED_TIMEOUT_SECS is 1800 with no env override.

WATCHDOG="${BATS_TEST_DIRNAME}/../../../scripts/autospec-watchdog.sh"

# ---------------------------------------------------------------------------
# Setup / teardown
# ---------------------------------------------------------------------------

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="testorg/testrepo"
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=10800
    export AUTOSPEC_WATCHDOG_STALE_SECS=1800
    export LABELS="$TEST_TMP/labels.txt"
    export COMMENTS="$TEST_TMP/comments.json"
    export CALLS="$TEST_TMP/calls.log"
    # worker-liveness.sh: "thishost" is this session's host; "crosshost" is another machine.
    export WORKER_LIVENESS_HOSTNAME="thishost"
    # Isolate the heartbeat pass: reconcile sees an empty issue list.
    : > "$TEST_TMP/issue-list.txt"
    export ISSUE_LIST="$TEST_TMP/issue-list.txt"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR"
    printf 'OPEN in-progress-by-bot\n' > "$LABELS"
    printf '[]\n' > "$COMMENTS"   # default: no run-state comment
    : > "$CALLS"

    # Sentinel: when present, gh fails only on --json comments (cross-check).
    GH_COMMENTS_FAIL_FILE="$TEST_TMP/gh_comments_should_fail"
    export GH_COMMENTS_FAIL_FILE

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
  if [ -n "${ISSUE_LIST:-}" ] && [ -f "${ISSUE_LIST}" ]; then
    cat "${ISSUE_LIST}"
  else
    printf '\n'
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  if printf '%s\n' "$*" | grep -q -- '--json comments'; then
    # T4: fail-safe gate — if sentinel exists, simulate an API error.
    if [ -f "${GH_COMMENTS_FAIL_FILE:-/dev/null/no}" ]; then
      exit 1
    fi
    issue="$3"
    filter=""; prev=""
    for a in "$@"; do
      [ "$prev" = "--jq" ] && filter="$a"
      prev="$a"
    done
    comments_for_issue="$(jq --argjson issue "$issue" \
      '[.[] | select(.number == $issue)]' "${COMMENTS:?}")"
    printf '{"comments":%s}\n' "$comments_for_issue" | jq -r "${filter:-.}"
  else
    cat "${LABELS:?}"
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  remove=""; add=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --remove-label) remove="$2"; shift 2 ;;
      --add-label)    add="$2";    shift 2 ;;
      *) shift ;;
    esac
  done
  if [ "$remove" = "in-progress-by-bot" ] && [ "$add" = "auto-implement" ]; then
    printf 'OPEN auto-implement\n' > "${LABELS:?}"
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf 'OPEN\n'
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

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Canonical owner__name heartbeat directory (production form for testorg/testrepo).
hb_dir() { printf '%s/testorg__testrepo' "$AUTOSPEC_WATCHDOG_DIR"; }

# Write a heartbeat file with an effective age of $3 seconds.
write_hb() {
    local issue="$1" step="$2" age_secs="$3"
    local dir; dir="$(hb_dir)"
    mkdir -p "$dir"
    local now ts
    now="$(date -u +%s)"
    ts=$((now - age_secs))
    printf '{"issue":"%s","branch":"feat/x","step":"%s","ts":%s,"pr":"","repo":"testorg/testrepo"}\n' \
        "$issue" "$step" "$ts" > "${dir}/${issue}.json"
}

# Write a run-state comment using a CROSS-HOST worker_id and the given updated_at.
# "crosshost" does not match WORKER_LIVENESS_HOSTNAME="thishost", so
# worker-liveness.sh returns "unknown" and falls through to GitHub-ts freshness.
write_state_comment() {
    local issue="$1" updated_at="$2"
    jq -n \
      --argjson issue "$issue" \
      --arg updated_at "$updated_at" \
      '[{
        number: $issue,
        createdAt: $updated_at,
        id: 1,
        body: ("<!-- autospec-run-state:begin -->\n" +
          ({schema:1,repo:"testorg/testrepo",issue:$issue,
            worker_id:"crosshost:ci:shell:12345",
            state:"claimed",branch:"feat/x",pr:"",step:"claimed",
            paths:[],claimed_at:$updated_at,updated_at:$updated_at,
            ttl_seconds:1800} | tojson) +
          "\n<!-- autospec-run-state:end -->")
      }]' > "$COMMENTS"
}

# ---------------------------------------------------------------------------
# T1 — age < 1800s: NOT released regardless of GitHub state.
# ---------------------------------------------------------------------------
@test "T1: claimed heartbeat younger than 1800s is NOT released" {
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=1800
    write_hb 42 claimed 60      # 60s old — well under the 1800s threshold

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(hb_dir)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
}

# ---------------------------------------------------------------------------
# T2 — age > 1800s BUT GitHub run-state is fresh: NOT released.
# ---------------------------------------------------------------------------
@test "T2: claimed heartbeat older than timeout with fresh GitHub run-state is NOT released" {
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=1800
    write_hb 42 claimed 2000    # 2000s old — past threshold, triggers cross-check
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_state_comment 42 "$fresh"   # GitHub ts = NOW → fresh → hold

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(hb_dir)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
    ! grep -q -- '--add-label auto-implement' "$CALLS"
}

# ---------------------------------------------------------------------------
# T3 — age > 1800s with stale/absent run-state: released.
# ---------------------------------------------------------------------------
@test "T3a: claimed heartbeat older than timeout with absent run-state comment is released" {
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=1800
    write_hb 42 claimed 2000    # past threshold
    printf '[]\n' > "$COMMENTS" # no run-state comment at all → reclaim

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(hb_dir)/42.json" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

@test "T3b: claimed heartbeat older than timeout with stale GitHub run-state is released" {
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=1800
    write_hb 42 claimed 2000
    stale="$(date -u -v-31M +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '31 minutes ago' +'%Y-%m-%dT%H:%M:%SZ')"
    write_state_comment 42 "$stale"   # stale GitHub ts (31 min ago > 1800s window) → reclaim

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(hb_dir)/42.json" ]
    grep -q -- '--add-label auto-implement' "$CALLS"
}

# ---------------------------------------------------------------------------
# T4 — gh exits non-zero during run-state cross-check: NOT released (fail-safe).
# ---------------------------------------------------------------------------
@test "T4: gh API failure during run-state cross-check is fail-safe — claimed heartbeat NOT released" {
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=1800
    write_hb 42 claimed 2000    # past threshold — would trigger reclaim without fail-safe
    touch "$GH_COMMENTS_FAIL_FILE"  # make only the --json comments call exit 1

    run bash "$WATCHDOG"

    # Watchdog itself exits 0 (handles errors gracefully).
    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(hb_dir)/42.json" ]  # heartbeat preserved — live claim is protected
    ! grep -q -- '--add-label auto-implement' "$CALLS"
}

# ---------------------------------------------------------------------------
# T5 — default WATCHDOG_CLAIMED_TIMEOUT_SECS is 1800 with no env override.
# ---------------------------------------------------------------------------
@test "T5: default WATCHDOG_CLAIMED_TIMEOUT_SECS is 1800 — heartbeat aged 1799s is NOT released" {
    # Do NOT export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS — exercise the built-in default.
    unset AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS
    write_hb 42 claimed 1799    # 1799s old: just under the 1800s default → must NOT release

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(hb_dir)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
}
