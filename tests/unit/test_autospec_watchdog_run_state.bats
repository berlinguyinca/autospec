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
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
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
  # Heartbeat-path tests set ISSUE_LIST to an explicit (possibly empty) file so
  # the reconcile pass can be isolated from the local-heartbeat pass. Default:
  # derive the in-progress list from the run-state comments fixture.
  if [ -n "${ISSUE_LIST:-}" ] && [ -f "${ISSUE_LIST}" ]; then
    cat "${ISSUE_LIST}"
  else
    jq -r '.[].number' "${COMMENTS:?}"
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  if printf '%s\n' "$*" | grep -q -- '--json comments'; then
    issue="$3"
    # Run the REAL --jq filter the production code passes (gh applies --jq
    # client-side after fetching --json comments). We reconstruct the
    # {comments:[...]} shape gh would return for this issue and pipe it through
    # the production filter verbatim, so the SUT's own selection logic — not a
    # re-implementation in this mock — is what the test exercises.
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

if [ "$1" = "api" ]; then
  path="$2"
  method="GET"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  case "$path $method" in
    repos/testorg/testrepo/issues/comments/*\ DELETE)
      id="${path##*/}"
      jq --argjson id "$id" '[.[] | select((.id // -1) != $id)]' "${COMMENTS:?}" > "${COMMENTS}.tmp"
      mv "${COMMENTS}.tmp" "${COMMENTS}"
      exit 0
      ;;
  esac
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

# ── F1+F2+F3: GitHub-authority + same-host PID-liveness reclaim gate ───────────
#
# These exercise the LOCAL HEARTBEAT path (the bare line-408 reclaim that caused
# go-modules #1055): a `claimed` heartbeat older than the timeout is only a
# TRIGGER; the GitHub `autospec-run-state` worker_id + PID-liveness decide.
# ISSUE_LIST is forced empty so only the heartbeat pass runs (isolating it from
# the reconcile pass, which would otherwise double-process the same issue).

# Heartbeat dir matches the CANONICAL owner__name slug that production writers
# (heartbeat-write.sh, SKILL.md, the watchdog flat-migration dest) now emit via
# repo-slug.sh — and that the watchdog reader resolves canonical-first. Before
# the F4 writer migration, production wrote the legacy single-underscore form
# while this fixture keyed canonical (a self-consistent-fixture gap); the
# `real heartbeat-write.sh` test below closes it end-to-end.
HB_DIR_FOR() { printf '%s/testorg__testrepo' "$AUTOSPEC_WATCHDOG_DIR"; }

REPO_SLUG_SH_PATH="${BATS_TEST_DIRNAME}/../../scripts/repo-slug.sh"
HEARTBEAT_WRITE_SH="${BATS_TEST_DIRNAME}/../../skills/autospec-run/scripts/heartbeat-write.sh"

write_hb() {
    local issue="$1" step="$2" age="$3"
    local dir; dir="$(HB_DIR_FOR)"
    mkdir -p "$dir"
    local now ts; now="$(date -u +%s)"; ts=$((now - age))
    printf '{"issue":"%s","branch":"feat/x","step":"%s","ts":%s,"pr":"","repo":"testorg/testrepo"}\n' \
        "$issue" "$step" "$ts" > "${dir}/${issue}.json"
}

# Run-state comment with an explicit worker_id and updated_at (host:user:harness:pid).
write_state_worker() {
    local issue="$1" state="$2" updated_at="$3" worker_id="$4"
    jq -n \
      --argjson issue "$issue" \
      --arg state "$state" \
      --arg updated_at "$updated_at" \
      --arg worker_id "$worker_id" \
      '[{
        number: $issue,
        body: ("<!-- autospec-run-state:begin -->\n" +
          ({schema:1,repo:"testorg/testrepo",issue:$issue,worker_id:$worker_id,state:$state,branch:"feat/x",pr:"",step:$state,paths:[],claimed_at:$updated_at,updated_at:$updated_at,ttl_seconds:300} | tojson) +
          "\n<!-- autospec-run-state:end -->")
      }]'
}

isolate_heartbeat_pass() {
    : > "$TEST_TMP/issue-list.txt"          # empty → reconcile pass is a no-op
    export ISSUE_LIST="$TEST_TMP/issue-list.txt"
    export WORKER_LIVENESS_HOSTNAME="thishost"
}

@test "regression #1055: live same-host pid past timeout with fresh claim is NOT reclaimed" {
    isolate_heartbeat_pass
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 1055 claimed 360
    write_state_worker 1055 claimed "$fresh" "thishost:me:shell:$$" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(HB_DIR_FOR)/1055.json" ]                       # heartbeat preserved
    grep -q 'in-progress-by-bot' "$LABELS"                 # label untouched
    ! grep -q -- '--add-label auto-implement' "$CALLS"
}

@test "dead same-host pid past timeout is reclaimed and a released comment is written" {
    isolate_heartbeat_pass
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 42 claimed 360
    # PID 999999 is not a live process on this host → worker-liveness = dead.
    write_state_worker 42 claimed "$fresh" "thishost:me:shell:999999" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/42.json" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
    grep -q 'released' "$CALLS"
}

@test "cross-host worker with stale GitHub ts past the window is reclaimed" {
    isolate_heartbeat_pass
    stale="$(date -u -v-10M +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '10 minutes ago' +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 42 claimed 360
    write_state_worker 42 claimed "$stale" "otherhost:me:shell:$$" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/42.json" ]
    grep -q -- '--add-label auto-implement' "$CALLS"
}

@test "cross-host worker with fresh GitHub ts is NOT reclaimed" {
    isolate_heartbeat_pass
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 42 claimed 360
    write_state_worker 42 claimed "$fresh" "otherhost:me:shell:$$" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=0"* ]]
    [ -f "$(HB_DIR_FOR)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
}

@test "absent run-state past timeout is reclaimed" {
    isolate_heartbeat_pass
    write_hb 42 claimed 360
    printf '[]\n' > "$COMMENTS"                            # no run-state comment

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/42.json" ]
    grep -q -- '--add-label auto-implement' "$CALLS"
}

# A marked run-state comment with a marker body, parametrized worker_id/ts/order.
state_comment_obj() {
    local issue="$1" state="$2" updated_at="$3" worker_id="$4" created_at="$5" id="$6"
    jq -n \
      --argjson issue "$issue" --arg state "$state" --arg updated_at "$updated_at" \
      --arg worker_id "$worker_id" --arg created_at "$created_at" --argjson id "$id" \
      '{
        number: $issue, createdAt: $created_at, id: $id,
        body: ("<!-- autospec-run-state:begin -->\n" +
          ({schema:1,repo:"testorg/testrepo",issue:$issue,worker_id:$worker_id,state:$state,branch:"feat/x",pr:"",step:$state,paths:[],claimed_at:$updated_at,updated_at:$updated_at,ttl_seconds:300} | tojson) +
          "\n<!-- autospec-run-state:end -->")
      }'
}

# ── F5: WATCHDOG_RECLAIM_SECS (3h) path — GitHub-authority gate (#1367) ─────────
#
# The bare 3h reclaim must also consult reclaim_decision so a live worker on a
# non-`claimed` step is never reclaimed without GitHub corroboration.
# Tests mirror the F1+F2+F3 pattern: heartbeat step="implementing", threshold
# overridden small, and STALE_SECS set below RECLAIM_SECS so the age=360 hb
# reaches the reclaim branch (not the "too fresh" early-continue).


@test "issue 1779: two watchdog reclaims clear stale run-state before the next queue read" {
    isolate_heartbeat_pass
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=3600
    export AUTOSPEC_WATCHDOG_STALE_SECS=10
    stale="$(date -u -v-2H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '2 hours ago' +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 1779 expand_start 7200
    jq -n --argjson comment "$(state_comment_obj 1779 expand_start "$stale" "oldhost:me:shell:1779" "2020-01-01T00:00:00Z" 177900)" '[$comment]' > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/1779.json" ]
    run jq '[.[] | select(.number == 1779 and ((.body // "") | contains("autospec-run-state:begin")))] | length' "$COMMENTS"
    [ "$output" = "0" ]

    # A second consecutive reclaim attempt for the same issue must not observe
    # or report the old worker_id after the first pass cleared the stale lease.
    printf 'OPEN in-progress-by-bot\n' > "$LABELS"
    write_hb 1779 expand_start 7200
    : > "$CALLS"
    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/1779.json" ]
    ! grep -q 'oldhost:me:shell:1779' "$CALLS"
}

@test "F5: live same-host pid on non-claimed step past 3h is NOT reclaimed" {
    isolate_heartbeat_pass
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=300
    export AUTOSPEC_WATCHDOG_STALE_SECS=10
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    # expand_start is a valid schema step (non-claimed) a worker may be on >3h.
    write_hb 42 expand_start 360
    write_state_worker 42 expand_start "$fresh" "thishost:me:shell:$$" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=0"* ]]
    [ -f "$(HB_DIR_FOR)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
    ! grep -q -- '--add-label auto-implement' "$CALLS"
}

@test "F5: gh failure on non-claimed step past 3h fail-safes to hold" {
    isolate_heartbeat_pass
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=300
    export AUTOSPEC_WATCHDOG_STALE_SECS=10
    write_hb 42 expand_start 360
    # Replace the gh stub so the run-state fetch returns non-zero (API down).
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "${CALLS:?}"
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then printf 'testorg/testrepo\n'; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  if [ -n "${ISSUE_LIST:-}" ] && [ -f "${ISSUE_LIST}" ]; then cat "${ISSUE_LIST}"; else jq -r '.[].number' "${COMMENTS:?}"; fi
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  if printf '%s\n' "$*" | grep -q -- '--json comments'; then
    exit 1   # simulate GitHub API failure → fail-safe to hold
  fi
  cat "${LABELS:?}"
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then cat "${PR_STATE:?}"; exit 0; fi
exit 0
SH
    chmod +x "$TEST_TMP/bin/gh"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=0"* ]]
    [ -f "$(HB_DIR_FOR)/42.json" ]
    grep -q 'in-progress-by-bot' "$LABELS"
}

@test "F5: absent run-state on non-claimed step past 3h is reclaimed" {
    isolate_heartbeat_pass
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=300
    export AUTOSPEC_WATCHDOG_STALE_SECS=10
    write_hb 42 expand_start 360
    printf '[]\n' > "$COMMENTS"   # no run-state comment → no authoritative owner

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/42.json" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

@test "F5: dead same-host pid on non-claimed step past 3h is reclaimed" {
    isolate_heartbeat_pass
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=300
    export AUTOSPEC_WATCHDOG_STALE_SECS=10
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 42 expand_start 360
    # PID 999999 is not alive on this host → worker_liveness = dead → reclaim.
    write_state_worker 42 expand_start "$fresh" "thishost:me:shell:999999" > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"reclaimed=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/42.json" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

# ── F4: canonical-writer ↔ canonical/legacy-reader contract (no split-brain) ──

@test "F4: real heartbeat-write.sh writes the canonical owner__name dir and the watchdog reads/reclaims it" {
    isolate_heartbeat_pass
    export AUTOSPEC_REPO_SLUG_SH="$REPO_SLUG_SH_PATH"
    # Age the trigger to 0 so a just-written heartbeat fires the reclaim path.
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=0

    # Use the REAL production writer — not a hand-built fixture — so the test
    # exercises the actual writer→reader keying contract end-to-end.
    bash "$HEARTBEAT_WRITE_SH" --issue 77 --step claimed --repo testorg/testrepo

    # The writer MUST land in the canonical owner__name dir, not legacy.
    [ -f "$(HB_DIR_FOR)/77.json" ]
    [ ! -d "$AUTOSPEC_WATCHDOG_DIR/testorg_testrepo" ]

    printf '[]\n' > "$COMMENTS"          # absent run-state → reclaim

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$(HB_DIR_FOR)/77.json" ]      # watchdog read the canonical dir + reclaimed
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

@test "F4: a legacy owner_name heartbeat (in-flight pre-migration) is STILL read and reclaimed via resolve_slug_dir" {
    isolate_heartbeat_pass
    # Only the legacy single-underscore dir exists (written by a pre-migration
    # worker). The canonical dir is absent, so resolve_slug_dir must fall back to
    # the legacy dir for one release instead of orphaning the live heartbeat.
    legacy_dir="$AUTOSPEC_WATCHDOG_DIR/testorg_testrepo"
    mkdir -p "$legacy_dir"
    now="$(date -u +%s)"; ts=$((now - 360))
    printf '{"issue":"%s","branch":"feat/x","step":"claimed","ts":%s,"pr":"","repo":"testorg/testrepo"}\n' \
        88 "$ts" > "$legacy_dir/88.json"
    [ ! -d "$(HB_DIR_FOR)" ]              # canonical dir does NOT exist

    printf '[]\n' > "$COMMENTS"          # absent run-state → reclaim

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    [ ! -f "$legacy_dir/88.json" ]        # legacy heartbeat was found + reclaimed
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$CALLS"
}

@test "duplicate run-state comments: oldest (CAS-authoritative) owner decides, not array order" {
    isolate_heartbeat_pass
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    write_hb 42 claimed 360
    # Array order puts the LOSER first (a live pid → would hold under naive [0]).
    # The CAS winner is the OLDEST/lowest-id comment, whose pid is dead → reclaim.
    # Reclaim proves the watchdog honored CAS order, not array order.
    jq -n \
      --argjson loser "$(state_comment_obj 42 claimed "$fresh" "thishost:me:shell:$$" "2024-06-01T00:00:00Z" 200)" \
      --argjson winner "$(state_comment_obj 42 claimed "$fresh" "thishost:me:shell:999999" "2020-01-01T00:00:00Z" 100)" \
      '[$loser, $winner]' > "$COMMENTS"

    run bash "$WATCHDOG"

    [ "$status" -eq 0 ]
    [[ "$output" == *"claimed_released=1"* ]]
    grep -q -- '--add-label auto-implement' "$CALLS"
}
