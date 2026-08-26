#!/usr/bin/env bats
# tests/autonomous/test_conductor_selfmerge_aftermath.bats — conductor
# self-merge aftermath in autospec_conductor_run()
# (docs/specs/2026-07-10-autonomous-integration-branch-design.md,
# §Architecture items 5 tail + 7, §Error handling).
#
# Covers:
#   1. rollup-update runs (sync then rollup-update, in order) after a
#      self-originated PR merges into the integration branch (mock
#      invocation log), with the landed issue/pr passed through.
#   2. A rollup-red stdout signal from rollup-update writes a durable pause
#      marker; on the NEXT cycle the self subset is parked (no dispatch,
#      code_health:self_originated_parked) while the operator subset still
#      dispatches to the parent.
#   3. Exceeding autonomous.self_originated.max_open_prs (via `status`)
#      parks the self subset + notifies; the operator subset is unaffected.
#   4. A post-merge `sync` exit 65 parks self-originated tiers, writes the
#      code_health:integration_sync_conflict marker, and notifies.
#   5. A clean rollup-update (no rollup-red, exit 0) writes no pause marker;
#      a nonzero rollup-update exit (e.g. a parked gh failure) ALSO writes a
#      pause marker, since a non-"rollup-red" failure must never silently
#      fall through to the pause-clearing branch (peer-review must-fix).
#   6. A Tier-1 cycle with no last-outcome.json (ordinary backlog work, no
#      self-originated merge yet) is a silent no-op — no sync/rollup-update
#      calls, no pause file written.
#
# Note: once self-originated-pause.json exists, the pre-dispatch gate parks
# the self subset BEFORE any dispatch/aftermath code runs — so an existing
# pause marker is never cleared by a later cycle's clean merge (by design:
# clearing requires an operator/control-channel action, out of this issue's
# scope per its "Out of scope: control-channel promote/discard" line).
#
# Mocking strategy mirrors tests/autonomous/test_conductor_provenance_dispatch.bats:
# helper scripts stubbed via CONDUCTOR_SCRIPTS_DIR; gh stubbed via a fake PATH
# dir; notify.sh resolved script-relative; bash 3.2-safe (no process
# substitution; fixtures written to real temp files). No real GitHub calls.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  mkdir -p "$HOME/.autospec"

  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS"
  export AUTOSPEC_QUEUE_BIN="$FAKE_SCRIPTS/autospec"
  cp "$REPO_ROOT/scripts/autospec-runtime-config.sh" "$FAKE_SCRIPTS/autospec-runtime-config.sh"

  FAKE_BIN="$TEST_TMP/fake-bin"
  mkdir -p "$FAKE_BIN"
  export PATH="$FAKE_BIN:$PATH"

  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "repo" ] && [ "${2:-}" = "view" ]; then
  case "$*" in
    *defaultBranchRef*) printf '%s\n' "${AUTOSPEC_TEST_DEFAULT_BRANCH:-main}" ;;
    *) echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  esac
elif [ "${1:-}" = "issue" ]; then
  echo "[]"
else
  exit 0
fi
EOF
  chmod +x "$FAKE_BIN/gh"

  NOTIFY_LOG="$TEST_TMP/notify.log"
  mkdir -p "$TEST_TMP/skills/autospec-shared/scripts"
  cat > "$TEST_TMP/skills/autospec-shared/scripts/notify.sh" <<EOF
#!/usr/bin/env bash
printf '%s | %s\n' "\${1:-}" "\${2:-}" >> "$NOTIFY_LOG"
exit 0
EOF
  chmod +x "$TEST_TMP/skills/autospec-shared/scripts/notify.sh"

  # Mode file location: _repo_root = parent of CONDUCTOR_SCRIPTS_DIR = TEST_TMP.
  MODE_FILE="$TEST_TMP/.autospec/explore-mode.json"
  PAUSE_FILE="$TEST_TMP/.autospec/self-originated-pause.json"
  OUTCOME_FILE="$TEST_TMP/.autospec/last-outcome.json"
  mkdir -p "$TEST_TMP/.autospec"

  RUN_CMD_LOG="$TEST_TMP/run-cmd.log"
  INT_CALL_LOG="$TEST_TMP/intbranch-calls.log"
  PROV_CALL_LOG="$TEST_TMP/prov-calls.log"
  WRITEBACK_LOG="$TEST_TMP/writeback-calls.log"
  touch "$RUN_CMD_LOG" "$INT_CALL_LOG" "$PROV_CALL_LOG" "$WRITEBACK_LOG"
  # No board configured unless a test opts in via _write_board_cache /
  # _install_board_writeback — matches every pre-existing test's posture.
  unset AUTOSPEC_BOARD_WRITEBACK_SCRIPT

  cat > "$TEST_TMP/run-cmd.sh" <<EOF
#!/usr/bin/env bash
if [ -f "$MODE_FILE" ]; then
  _kind="\$(jq -r '.kind // "none"' "$MODE_FILE" 2>/dev/null || echo parse-error)"
else
  _kind="absent"
fi
printf 'issues=%s kind=%s\n' "\${AUTOSPEC_RUN_ONLY_ISSUES:-}" "\$_kind" >> "$RUN_CMD_LOG"
EOF
  chmod +x "$TEST_TMP/run-cmd.sh"
  export AUTOSPEC_RUN_CMD="bash $TEST_TMP/run-cmd.sh"

  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN \
    MODE_FILE PAUSE_FILE OUTCOME_FILE RUN_CMD_LOG INT_CALL_LOG PROV_CALL_LOG NOTIFY_LOG \
    WRITEBACK_LOG
}

teardown() {
  rm -rf "$TEST_TMP" 2>/dev/null || true
}

_install_stub() {
  local name="$1"
  local body="$2"
  printf '#!/usr/bin/env bash\n%s\n' "$body" > "$FAKE_SCRIPTS/$name"
  chmod +x "$FAKE_SCRIPTS/$name"
}

_install_common_stubs() {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
}

_install_queue() {
  local numbers_csv="$1"
  cat > "$FAKE_SCRIPTS/autospec" <<EOF
#!/usr/bin/env bash
shift 2
jq -cn '{
  ready: ([$numbers_csv] | map({number: .})),
  blocked: [], claimed: [], conflicts: [],
  worker_cap: {max_repo_workers: 0, active_count: 0, remaining: 0, reached: false},
  batch: ([$numbers_csv] | map({number: .}))
}'
EOF
  chmod +x "$FAKE_SCRIPTS/autospec"
}

# provenance mock: issues 1xx -> self, 2xx -> operator.
_install_provenance() {
  cat > "$FAKE_SCRIPTS/autonomous-provenance.sh" <<EOF
#!/usr/bin/env bash
issue=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    --issue) shift; issue="\${1:-}" ;;
  esac
  shift
done
printf 'resolve %s\n' "\$issue" >> "$PROV_CALL_LOG"
case "\$issue" in
  1*) printf 'self\n' ;;
  2*) printf 'operator\n' ;;
  *)  printf 'operator\n' ;;
esac
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-provenance.sh"
}

# integration-branch mock: logs args.
#   ensure  -> writes kind=integration mode file (mirrors real script).
#   sync    -> exits with rc recorded in $TEST_TMP/sync-rc (default 0).
#   status  -> emits JSON from $TEST_TMP/status-json (default: caps all clear).
#   rollup-update -> prints "rollup-red\n" iff $TEST_TMP/rollup-red exists.
_install_intbranch() {
  cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INT_CALL_LOG"
case "\${1:-}" in
  ensure)
    mkdir -p "$TEST_TMP/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"test-owner/test-repo","base":"main","head_sha":"abc123","kind":"integration"}\n' > "$MODE_FILE"
    ;;
  sync)
    rc="\$(cat "$TEST_TMP/sync-rc" 2>/dev/null || printf '0')"
    exit "\$rc"
    ;;
  status)
    if [ -f "$TEST_TMP/status-json" ]; then
      cat "$TEST_TMP/status-json"
    else
      printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":null,"state":null},"accumulated_pr_count":1,"age_days":1,"diff_lines":10}\n'
    fi
    ;;
  rollup-update)
    if [ -f "$TEST_TMP/rollup-red" ]; then
      printf 'rollup-red\n'
    fi
    ;;
esac
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"
}

# Board-cache fixture consumed by _autospec_conductor_board_state (mapping
# repo/issue -> item_id), dropped into $HOME/.autospec/board-cache (HOME ==
# TEST_TMP per setup()).
_write_board_cache() {
  local repo="$1" issue="$2" item_id="$3"
  mkdir -p "$TEST_TMP/.autospec/board-cache"
  cat > "$TEST_TMP/.autospec/board-cache/plan.json" <<EOF
{"items":[{"repo":"$repo","number":$issue,"item_id":"$item_id"}]}
EOF
}

# project-board-writeback.sh stub: logs every invocation to WRITEBACK_LOG.
_install_board_writeback() {
  cat > "$FAKE_SCRIPTS/project-board-writeback.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$WRITEBACK_LOG"
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/project-board-writeback.sh"
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$FAKE_SCRIPTS/project-board-writeback.sh"
}

_run_cycle() {
  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=1 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1
}

# ── 1. rollup-update runs after a self-originated merge ──────────────────────

@test "self-originated merge: sync then rollup-update run in order with issue/pr" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"

  _run_cycle

  [ "$status" -eq 0 ]
  # sync (dispatch-time) + sync (aftermath) + rollup-update, in that relative order.
  grep -n '^rollup-update --parent main --issue 101 --pr 501' "$INT_CALL_LOG"
  # rollup-update must appear AFTER the last sync call.
  last_sync_line="$(grep -n '^sync --parent main' "$INT_CALL_LOG" | tail -1 | cut -d: -f1)"
  rollup_line="$(grep -n '^rollup-update --parent main --issue 101 --pr 501' "$INT_CALL_LOG" | cut -d: -f1)"
  [ "$rollup_line" -gt "$last_sync_line" ]
}

@test "self-originated merge: clean rollup-update (no rollup-red) writes no pause marker" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"

  _run_cycle

  [ "$status" -eq 0 ]
  [ ! -f "$PAUSE_FILE" ]
}

@test "rollup-update nonzero exit (non-rollup-red failure) still parks self-originated tiers" {
  _install_common_stubs
  _install_provenance
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"

  cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INT_CALL_LOG"
case "\${1:-}" in
  ensure)
    mkdir -p "$TEST_TMP/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"test-owner/test-repo","base":"main","head_sha":"abc123","kind":"integration"}\n' > "$MODE_FILE"
    ;;
  sync)
    exit 0
    ;;
  rollup-update)
    echo "rollup PR create failed twice" >&2
    exit 8
    ;;
esac
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$PAUSE_FILE" ]
  [ "$(jq -r '.reason' "$PAUSE_FILE")" = "rollup_update_failed" ]
  grep -q 'rollup-update failed' "$NOTIFY_LOG"
}

# ── 2. rollup-red pauses self-originated merges; operator tier proceeds ──────

@test "rollup-red: writes a durable pause marker and notifies" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  : > "$TEST_TMP/rollup-red"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$PAUSE_FILE" ]
  [ "$(jq -r '.reason' "$PAUSE_FILE")" = "rollup_red" ]
  grep -q 'roll-up CI red' "$NOTIFY_LOG"
}

@test "pause marker present: self subset parked (no dispatch), operator subset still dispatches" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101,202"
  printf '{"reason":"rollup_red"}\n' > "$PAUSE_FILE"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"code_health:self_originated_parked"* ]]
  grep -q '^issues=202 kind=absent$' "$RUN_CMD_LOG"
  ! grep -q 'issues=101' "$RUN_CMD_LOG"
  grep -q 'self-originated tiers parked' "$NOTIFY_LOG"
}

# ── 3. Caps exceeded parks self tiers + notifies; operator unaffected ────────

@test "max_open_prs exceeded: self subset parked + notified, operator subset dispatches" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101,202"
  printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":9,"state":"OPEN"},"accumulated_pr_count":21,"age_days":1,"diff_lines":10}\n' \
    > "$TEST_TMP/status-json"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"code_health:self_originated_parked"* ]]
  grep -q '^issues=202 kind=absent$' "$RUN_CMD_LOG"
  ! grep -q 'issues=101' "$RUN_CMD_LOG"
  grep -q 'self-originated tiers parked: max_open_prs' "$NOTIFY_LOG"
}

@test "caps clear: self subset dispatches normally" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":null,"state":null},"accumulated_pr_count":2,"age_days":1,"diff_lines":10}\n' \
    > "$TEST_TMP/status-json"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues=101 kind=integration$' "$RUN_CMD_LOG"
}

# ── 4. Post-merge sync exit 65 parks self tiers + code_health marker ─────────

@test "post-merge sync exit 65: parks self-originated tiers with code_health marker + notify" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"

  # First sync call (dispatch-time ensure+sync) succeeds; the aftermath's
  # post-merge sync call is the SECOND invocation of `sync` this cycle. The
  # rc file is read fresh each call, so flip it to 65 after the first read
  # by using a counter script instead of a static rc file.
  cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INT_CALL_LOG"
case "\${1:-}" in
  ensure)
    mkdir -p "$TEST_TMP/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"test-owner/test-repo","base":"main","head_sha":"abc123","kind":"integration"}\n' > "$MODE_FILE"
    ;;
  sync)
    _n=\$(cat "$TEST_TMP/sync-calls" 2>/dev/null || echo 0)
    _n=\$((_n + 1))
    echo "\$_n" > "$TEST_TMP/sync-calls"
    if [ "\$_n" -ge 2 ]; then
      exit 65
    fi
    ;;
  status)
    printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":null,"state":null},"accumulated_pr_count":1,"age_days":1,"diff_lines":10}\n'
    ;;
esac
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"code_health:integration_sync_conflict"* ]]
  [ -f "$PAUSE_FILE" ]
  [ "$(jq -r '.reason' "$PAUSE_FILE")" = "sync_conflict" ]
  grep -q 'post-merge sync conflict' "$NOTIFY_LOG"
  # rollup-update must NOT run when the post-merge sync conflicted.
  ! grep -q '^rollup-update' "$INT_CALL_LOG"
}

# ── 6. No outcome file: silent no-op ──────────────────────────────────────────

@test "no last-outcome.json: no aftermath sync/rollup-update calls, no pause file" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q '^issues=101 kind=integration$' "$RUN_CMD_LOG"
  ! grep -q '^rollup-update' "$INT_CALL_LOG"
  [ ! -f "$PAUSE_FILE" ]
}

# ---------------------------------------------------------------------------
# Board write-back (project-board-fleet-execution Plan B Task 5): a
# successful rollup-update is the real "issue rolled into the integration
# branch / its roll-up PR exists" moment — fires board state Review for the
# landed issue. Decorative only, must never affect the aftermath path.
# ---------------------------------------------------------------------------

@test "clean rollup-update fires board Review for the landed issue" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  _install_board_writeback
  _write_board_cache "test-owner/test-repo" 101 "PVTI_landed"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q -- "--item PVTI_landed --state Review" "$WRITEBACK_LOG"
}

@test "rollup-red still fires board Review (issue is in review; red is a separate signal)" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  : > "$TEST_TMP/rollup-red"
  _install_board_writeback
  _write_board_cache "test-owner/test-repo" 101 "PVTI_red"

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q -- "--item PVTI_red --state Review" "$WRITEBACK_LOG"
}

@test "rollup-update failure (nonzero exit) fires no board Review call" {
  _install_common_stubs
  _install_provenance
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  _install_board_writeback
  _write_board_cache "test-owner/test-repo" 101 "PVTI_neverlanded"

  cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INT_CALL_LOG"
case "\${1:-}" in
  ensure)
    mkdir -p "$TEST_TMP/.autospec"
    printf '{"branch":"autospec/autonomous-main","slug":"test-owner/test-repo","base":"main","head_sha":"abc123","kind":"integration"}\n' > "$MODE_FILE"
    ;;
  sync)
    exit 0
    ;;
  status)
    printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":null,"state":null},"accumulated_pr_count":1,"age_days":1,"diff_lines":10}\n'
    ;;
  rollup-update)
    exit 7
    ;;
esac
exit 0
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$PAUSE_FILE" ]
  run cat "$WRITEBACK_LOG"
  [ -z "$output" ]
}

@test "no board configured: self-originated merge aftermath runs cleanly with zero write-back calls" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  _install_board_writeback
  # Deliberately no _write_board_cache call: no board-cache dir exists.

  _run_cycle

  [ "$status" -eq 0 ]
  grep -q -- "^rollup-update --parent main --issue 101 --pr 501" "$INT_CALL_LOG"
  run cat "$WRITEBACK_LOG"
  [ -z "$output" ]
}

@test "CONDUCTOR_DRY_RUN=1: self-originated merge aftermath fires zero board write-back calls" {
  _install_common_stubs
  _install_provenance
  _install_intbranch
  _install_queue "101"
  printf '{"issue":101,"pr":501,"outcome":"merged","self_originated":true}\n' > "$OUTCOME_FILE"
  _install_board_writeback
  _write_board_cache "test-owner/test-repo" 101 "PVTI_dryrun"

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=1 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=1 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [ "$status" -eq 0 ]
  run cat "$WRITEBACK_LOG"
  [ -z "$output" ]
}
