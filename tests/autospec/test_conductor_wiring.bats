#!/usr/bin/env bats
# tests/autospec/test_conductor_wiring.bats
# Coverage for autospec_conductor_run() in scripts/lib/autospec-loop.sh (issue #1378).
#
# Tests:
#   1. One cycle selects via waterfall and runs the gate before drain.
#   2. Digest renders once per UTC day, not twice in same UTC day.
#   3. Park from spend-ledger writes resume context and arms ScheduleWakeup/cron.
#
# All gh calls, helper scripts, and notify.sh are stubbed via a fake PATH
# directory so no real GitHub calls or desktop notifications are emitted.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

  # Per-test isolated temp directory.
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  mkdir -p "$HOME/.autospec"

  # Fake scripts directory that holds stub helper scripts.
  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS"
  cp "$REPO_ROOT/scripts/autospec-runtime-config.sh" "$FAKE_SCRIPTS/autospec-runtime-config.sh"

  # Fake PATH so every helper call hits our stubs.
  FAKE_BIN="$TEST_TMP/fake-bin"
  mkdir -p "$FAKE_BIN"
  export PATH="$FAKE_BIN:$PATH"

  # Install a passive gh stub (avoid real GitHub calls).
  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
# stub gh — returns empty list for issue queries
case "${1:-}" in
  issue) echo "[]" ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  # Install a passive notify.sh stub.
  cat > "$FAKE_BIN/notify.sh" <<'EOF'
#!/usr/bin/env bash
printf 'notify: %s — %s\n' "${1:-}" "${2:-}" >&2
exit 0
EOF
  chmod +x "$FAKE_BIN/notify.sh"

  # Source the loop lib into the subshell used by each @test.
  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN
}

teardown() {
  rm -rf "$TEST_TMP" 2>/dev/null || true
}

# ── Helper: install a stub script in FAKE_SCRIPTS ────────────────────────────
_install_stub() {
  local name="$1"
  local body="$2"
  printf '#!/usr/bin/env bash\n%s\n' "$body" > "$FAKE_SCRIPTS/$name"
  chmod +x "$FAKE_SCRIPTS/$name"
}

# ── 1. One cycle: waterfall selects Tier 1, gate must emit merge-ok before drain ─
@test "conductor: single cycle calls waterfall then gate before drain" {
  # Stub control-channel → no decisions.
  _install_stub "autonomous-control-channel.sh" \
    'exit 0'

  # Stub waterfall → Tier 1 run-backlog.
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''

  # Stub premerge-gate → merge-ok.
  local gate_log="$TEST_TMP/gate.log"
  _install_stub "autonomous-premerge-gate.sh" \
    "printf 'merge-ok\n'; printf 'gate-called\n' >> \"$gate_log\""

  # Stub spend-ledger → continue.
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'

  # Stub resilience → passthrough.
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'

  # Stub autospec-usage-limit.sh.
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  # AUTOSPEC_RUN_CMD echoes what would run.
  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'autospec-run-called\n' >> '$run_log'"

  # Run one cycle with MAX_CYCLES=1, no sleep, no digest, dry-run off.
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

  # Gate must have been called (merge-ok verdict enables the drain).
  [ -f "$gate_log" ]
  grep -q 'gate-called' "$gate_log"

  # autospec-run-cmd must have been invoked (work was done after gate passed).
  [ -f "$run_log" ]
  grep -q 'autospec-run-called' "$run_log"
}

# ── 1b. Runtime PATH includes ~/.autospec/bin for installed helper commands ──
@test "conductor: prefixes ~/.autospec/bin onto PATH before gate runs" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''

  mkdir -p "$HOME/.autospec/bin"
  local path_log="$TEST_TMP/path.log"
  _install_stub "autonomous-premerge-gate.sh" \
    "case \":\$PATH:\" in *\":\$HOME/.autospec/bin:\"*) printf 'merge-ok\n'; printf 'path-ok\n' >> \"$path_log\";; *) printf 'halt missing-path\n'; exit 2;; esac"

  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'autospec-run-called\n' >> '$run_log'"

  run bash -c "
    export PATH='$FAKE_BIN:/usr/bin:/bin'
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=1 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [ -f "$path_log" ]
  grep -q 'path-ok' "$path_log"
}

# ── 1c. Phase-1 discovery park exits without sandbox/explore side effects ────
@test "conductor: park action exits without running backlog or discovery" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"park","reason":"discovery tiers disabled in Phase 1"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"
  export AUTOSPEC_EXPLORE_CMD="printf 'should-not-explore\n' >> '$run_log'"

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=5 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [[ "$output" == *"parking"* ]]
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-' "$run_log"
  fi
}

@test "conductor: empty Tier-1 queue skips drain and spend increment" {
  _install_stub "autonomous-control-channel.sh" \
    'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "list-ready-issues.sh" \
    'printf '"'"'{"ready":[],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'"'"''

  local gate_log="$TEST_TMP/gate.log"
  _install_stub "autonomous-premerge-gate.sh" \
    "printf 'merge-ok\n'; printf 'gate-called\n' >> \"$gate_log\""

  local spend_log="$TEST_TMP/spend.log"
  _install_stub "autonomous-spend-ledger.sh" \
    "case \"\${1:-}\" in add) printf 'spend-add\n' >> '$spend_log';; check) printf 'continue\n';; *) exit 0;; esac"

  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"

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

  [ "$status" -eq 0 ]
  [ ! -f "$gate_log" ]
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-run' "$run_log"
  fi
  if [ -f "$spend_log" ]; then
    ! grep -q 'spend-add' "$spend_log"
  fi
  [[ "$output" == *"Tier-1 queue empty"* ]]
}

# ── 2. Gate blocks → drain is skipped, no run invocation ─────────────────────
@test "conductor: gate block skips drain in that cycle" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" \
    "printf 'block test-reason\n'; exit 1"
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"

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

  # Run command must NOT have been invoked when gate blocks.
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-run' "$run_log"
  fi
}

# ── 3. Digest renders once per UTC day, not twice in same day ─────────────────
@test "conductor: digest renders once per UTC day" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'run\n' >> '$run_log'"

  # Run 3 cycles — digest should appear at most once (same UTC day).
  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=3 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=0 \
    autospec_conductor_run
  " 2>&1

  # Digest file should exist after 3 cycles (written on first cycle's day change).
  local repo_root="$FAKE_SCRIPTS/.."
  local digest="$FAKE_SCRIPTS/../.autospec/autonomous-digest.md"
  # Accept either the FAKE_SCRIPTS-relative path or check output for digest indication.
  # Conductor writes to repo_root/.autospec/autonomous-digest.md.
  # In test: CONDUCTOR_SCRIPTS_DIR=FAKE_SCRIPTS → repo_root = FAKE_SCRIPTS/..
  [ -f "$TEST_TMP/fake-scripts/../.autospec/autonomous-digest.md" ] || \
    [[ "$output" == *"digest"* ]]

  # Digest line appears at most once (one write, not three).
  local digest_count
  digest_count="$(printf '%s\n' "$output" | grep -c 'digest written' 2>/dev/null)" || digest_count=0
  [ "${digest_count:-0}" -le 1 ]
}

# ── 4. Spend-ledger park writes resume context and arms ScheduleWakeup/cron ──
@test "conductor: spend-ledger park arms resume and exits" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'

  # Stub spend-ledger → park on check.
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in
       add)   exit 0 ;;
       check) printf "park lifetime token cap reached (100 >= 100)\n" ;;
       *)     exit 0 ;;
     esac'

  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'

  # Stub usage-limit: record that arm was called.
  local arm_log="$TEST_TMP/arm.log"
  _install_stub "autospec-usage-limit.sh" \
    "case \"\${1:-}\" in arm) printf 'usage-limit-armed\n' >> \"$arm_log\";; *) exit 0;; esac"

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'run\n' >> '$run_log'"

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=10 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  # Loop must have exited (MAX_CYCLES was 10 but park triggers early exit).
  # Check that usage-limit arm was called (resume context armed).
  [ -f "$arm_log" ]
  grep -q 'usage-limit-armed' "$arm_log"

  # Output should mention park.
  [[ "$output" == *"park"* ]] || [[ "$output" == *"spend-ledger"* ]]
}

# ── 4b. Dry-run must not mutate persistent spend-ledger totals ───────────────
@test "conductor: dry-run does not call spend-ledger add" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'

  cp "$REPO_ROOT/scripts/autonomous-spend-ledger.sh" "$FAKE_SCRIPTS/autonomous-spend-ledger.sh"
  chmod +x "$FAKE_SCRIPTS/autonomous-spend-ledger.sh"

  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; main-health) printf "DECISION:continue\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"

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
  run bash "$REPO_ROOT/scripts/autonomous-spend-ledger.sh" status --repo-dir "$REPO_ROOT"
  [ "$status" -eq 0 ]
  run jq -r '.issues' <<<"$output"
  [ "$output" = "0" ]
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-run' "$run_log"
  fi
}

# ── 5. Missing gate script → halt with code_health identifier ─────────────────
@test "conductor: missing premerge-gate emits code_health halt and exits" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  # NO premerge-gate installed — deliberately absent.
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'run\n' >> '$run_log'"

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

  # Must emit code_health:autonomous_gate_missing.
  [[ "$output" == *"code_health:autonomous_gate_missing"* ]]

  # autospec-run must NOT have been invoked.
  if [ -f "$run_log" ]; then
    ! grep -q 'run' "$run_log"
  fi
}

# ── 6. Control-channel stop signal → loop exits gracefully ────────────────────
@test "conductor: control-channel graceful-stop exits after current cycle" {
  _install_stub "autonomous-control-channel.sh" \
    'printf "DECISION:graceful-stop\n"'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=10 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  # Must stop with control:graceful-stop reason.
  [[ "$output" == *"graceful-stop"* ]] || [[ "$output" == *"control"* ]]
}


@test "conductor: SIGTERM writes stopped marker, terminal state, and releases lock" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    "printf '%s\n' '{\"tier\":1,\"action\":\"run-backlog\",\"reason\":\"test\"}'"
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'

  local resilience_log="$TEST_TMP/resilience.log"
  cat > "$FAKE_SCRIPTS/autonomous-resilience.sh" <<EOF
#!/usr/bin/env bash
case "\${1:-} \${2:-}" in
  'lock acquire') printf 'lock:acquire\n' >> '$resilience_log'; printf 'DECISION:lock-acquired\nLOCK_SESSION:test\n' ;;
  'lock release') printf 'lock:release\n' >> '$resilience_log'; printf 'DECISION:lock-released\n' ;;
  'state write')
    shift 2
    status=''
    while [ "\$#" -gt 0 ]; do
      case "\$1" in --status) status="\$2"; shift 2 ;; *) shift ;; esac
    done
    printf 'state:%s\n' "\$status" >> '$resilience_log'
    printf 'DECISION:state-written\n'
    ;;
  *) exit 0 ;;
esac
EOF
  chmod +x "$FAKE_SCRIPTS/autonomous-resilience.sh"
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  export AUTOSPEC_RUN_CMD='kill -TERM "$PPID"; sleep 1'

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=0 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [ "$status" -eq 143 ]
  [[ "$output" == *"[conductor] stopped: signal:TERM (cycle=1)"* ]]
  grep -q 'state:stopped:signal:TERM:cycle-1' "$resilience_log"
  grep -q 'lock:release' "$resilience_log"
}

# ── 7. autospec_conductor_run exists in the loop lib ─────────────────────────
@test "conductor: autospec_conductor_run() is defined in scripts/lib/autospec-loop.sh" {
  run grep -c '^autospec_conductor_run()' "$LOOP_LIB"
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}

# ── 8. premerge-gate reference exists in loop lib ─────────────────────────────
@test "conductor: autospec-loop.sh references autonomous-premerge-gate" {
  run grep -c 'autonomous-premerge-gate\|premerge.gate\|premerge_gate' "$LOOP_LIB"
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}

# ── 9. spend-ledger reference exists in loop lib ──────────────────────────────
@test "conductor: autospec-loop.sh references autonomous-spend-ledger" {
  run grep -c 'autonomous-spend-ledger\|spend.ledger\|spend_ledger' "$LOOP_LIB"
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}

# ── 10. bash -n syntax clean ──────────────────────────────────────────────────
@test "conductor: scripts/lib/autospec-loop.sh passes bash -n" {
  run bash -n "$LOOP_LIB"
  [ "$status" -eq 0 ]
}

# ── 11. Main-health red → drain skipped, Tier-1 merges halt ──────────────────
# Phase 5.5 integration fix (#1380): the conductor MUST poll main-health and
# never drain onto a red main.  Prior to the fix, autospec_conductor_run() never
# invoked autonomous-resilience.sh main-health, so a red main did not halt
# Tier-1 merges (spec Phase-1 safety invariant).
@test "conductor: main-health red halts Tier-1 drain" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  # Resilience stub: main-health returns DECISION:halt (red main).
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in
       state) printf "DECISION:state-written\n" ;;
       lock)  printf "DECISION:lock-acquired\nLOCK_SESSION:test\n" ;;
       main-health) printf "DECISION:halt\nCI_STATE:failure\n"; exit 1 ;;
       *) exit 0 ;;
     esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=2 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  # Drain must NOT have run (no merging onto a red main).
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-run' "$run_log"
  fi
  # Loop must report the main-health halt stop reason.
  [[ "$output" == *"main-health"* ]]
}

# ── 12. Main-health pending → drain skipped this cycle (no halt) ─────────────
@test "conductor: main-health pending skips drain without halting" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '"'"'{"tier":1,"action":"run-backlog","reason":"test"}\n'"'"''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in
       state) printf "DECISION:state-written\n" ;;
       lock)  printf "DECISION:lock-acquired\nLOCK_SESSION:test\n" ;;
       main-health) printf "DECISION:wait\nCI_STATE:pending\n" ;;
       *) exit 0 ;;
     esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local run_log="$TEST_TMP/run.log"
  export AUTOSPEC_RUN_CMD="printf 'should-not-run\n' >> '$run_log'"

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

  # Drain skipped while main is pending.
  if [ -f "$run_log" ]; then
    ! grep -q 'should-not-run' "$run_log"
  fi
}

@test "conductor: Tier 1.5 promotion runs before parking when backlog is empty" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1.5,"action":"promote-open-issues","reason":"open issues"}\n'\'''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local promote_log="$TEST_TMP/promote.log"
  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD="printf '{\"dry\":false,\"filed\":2}\n'; printf 'promote-called\n' >> '$promote_log'"

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

  [ "$status" -eq 0 ]
  [ -f "$promote_log" ]
  grep -q 'promote-called' "$promote_log"
  [[ "$output" == *"Tier 1.5 promotion result: dry=false filed=2"* ]]
}

@test "conductor: Tier 3 architecture improvement command files work and floats to Tier 1" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":3,"action":"run-architecture-improvement","reason":"coverage dry"}\n'\'''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local arch_log="$TEST_TMP/arch.log"
  export AUTOSPEC_ARCHITECTURE_IMPROVEMENT_CMD="printf '{\"dry\":false,\"filed\":1}\n'; printf 'arch-called\n' >> '$arch_log'"

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

  [ "$status" -eq 0 ]
  [ -f "$arch_log" ]
  grep -q 'arch-called' "$arch_log"
  [[ "$output" == *"Tier 3 architecture result: dry=false filed=1"* ]]
}

# ── 20. Waterfall's Tier-1 gate receives the readiness-aware count, not the
#        naive open-auto-implement count (#1632: blocked-backlog livelock) ───
@test "conductor: waterfall receives readiness-aware backlog-count matching the drain queue" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'

  # Naive gh open auto-implement count is 1 (all blocked) — must NOT be what
  # the waterfall is gated on.
  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  issue) echo '[{"number":42}]' ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  # Dependency-aware list-ready-issues.sh: nothing ready, one blocked.
  _install_stub "list-ready-issues.sh" \
    'printf '"'"'{"ready":[],"blocked":[{"number":42}],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'"'"''

  # waterfall stub: record the args it was invoked with, then behave like the
  # real script would for a readiness-aware backlog-count of 0.
  local waterfall_args_log="$TEST_TMP/waterfall-args.log"
  _install_stub "autonomous-waterfall.sh" \
    "printf '%s\n' \"\$*\" >> '$waterfall_args_log'; printf '{\"tier\":1.5,\"action\":\"promote-open-issues\",\"reason\":\"test\"}\n'"

  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'

  local arch_log="$TEST_TMP/arch.log"
  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD="printf '{\"dry\":false,\"filed\":1}\n'; printf 'promote-called\n' >> '$arch_log'"

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

  [ "$status" -eq 0 ]
  [ -f "$waterfall_args_log" ]
  # The naive gh count (1) must NOT have been passed — the loop injected the
  # dependency-aware ready count (0: all blocked) instead.
  grep -q -- '--backlog-count 0' "$waterfall_args_log"
}

# ── 21. N consecutive all-blocked Tier-1 cycles must escalate past Tier 1 ────
# Regression guard for the observed livelock: a conductor whose only
# auto-implement issue is dependency-blocked must NOT spin tier=1 forever.
# Uses the REAL waterfall script so the wiring is exercised end-to-end.
@test "conductor: consecutive all-blocked cycles escalate past Tier 1 (no livelock)" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'

  # gh's `--json ... --jq 'length'`/`--jq '[...] | length'` flags are applied
  # by the real `gh` binary itself, so real callers always get back a bare
  # scalar count — not the raw JSON array. Mirror that here: the naive
  # open-auto-implement count is misleadingly 1 (the sole issue exists but is
  # dependency-blocked), which is exactly the condition #1632 fixes.
  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  issue) echo '1' ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_stub "list-ready-issues.sh" \
    'printf '"'"'{"ready":[],"blocked":[{"number":42}],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'"'"''

  cp "$REPO_ROOT/scripts/autonomous-waterfall.sh" "$FAKE_SCRIPTS/autonomous-waterfall.sh"
  chmod +x "$FAKE_SCRIPTS/autonomous-waterfall.sh"

  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD="printf '{\"dry\":true,\"filed\":0}\n'"

  # 5 cycles: default AUTOSPEC_AUTO_DRY_CYCLES threshold is 2, so cycle 3+
  # must show a tier other than bare "1" (dry cycles 0 and 1 are still logged
  # as tier=1 — that is expected/correct; the livelock is an UNBOUNDED run of
  # tier=1 that never advances).
  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=5 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [ "$status" -eq 0 ]
  # Count how many of the (at most 5) "tier=" lines are bare tier=1.
  tier1_lines="$(printf '%s\n' "$output" | grep -c '^\[conductor\] tier=1 ' || true)"
  total_tier_lines="$(printf '%s\n' "$output" | grep -c '^\[conductor\] tier=' || true)"
  [ "$total_tier_lines" -ge 1 ]
  # Must NOT be all tier=1 — the run must show at least one non-1 tier
  # (1.5/2/3/4) or a park, proving the cascade was reached.
  [ "$tier1_lines" -lt "$total_tier_lines" ]
}

@test "conductor: all-blocked backlog is logged distinctly and escalated for humans after dry promotion" {
  local gh_log="$TEST_TMP/gh.log"
  cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$gh_log"
case "\${1:-}" in
  issue) echo '1' ;;
  label) exit 0 ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "list-ready-issues.sh" \
    'printf '"'"'{"ready":[],"blocked":[{"number":42,"reason":"blocked_cycle","unmet_dependencies":[7]}],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'"'"''

  cp "$REPO_ROOT/scripts/autonomous-waterfall.sh" "$FAKE_SCRIPTS/autonomous-waterfall.sh"
  chmod +x "$FAKE_SCRIPTS/autonomous-waterfall.sh"

  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD="printf '{\"dry\":true,\"filed\":0,\"reason\":\"no-promotable-issues\"}\n'"

  run bash -c "
    . '$LOOP_LIB'
    CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
    CONDUCTOR_REPO='test-owner/test-repo' \
    CONDUCTOR_MAX_CYCLES=4 \
    CONDUCTOR_POLL_INTERVAL=0 \
    CONDUCTOR_DRY_RUN=0 \
    CONDUCTOR_NO_DIGEST=1 \
    autospec_conductor_run
  " 2>&1

  [ "$status" -eq 0 ]
  [[ "$output" == *"Tier-1 all-blocked (1 issues)"* ]]
  [[ "$output" == *"Tier 1.5 promotion result: dry=true filed=0"* ]]
  [[ "$output" == *"autospec:needs-human"* ]]
  grep -q 'label create autospec:needs-human' "$gh_log"
  grep -q 'issue edit 42 --repo test-owner/test-repo --add-label autospec:needs-human' "$gh_log"
}

# ── 22. Transient list-ready-issues.sh failure must NOT inject --backlog-count 0
#        (peer-review must-fix #1632): a helper blip must not masquerade as an
#        empty backlog — omit the flag so the waterfall's own readiness-aware
#        count (with naive-gh fallback) still runs. ───────────────────────────
@test "conductor: list-ready-issues.sh failure omits --backlog-count (no forced-0)" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'

  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  issue) echo '1' ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  # Helper fails (transient): exits non-zero, emits nothing parseable.
  _install_stub "list-ready-issues.sh" \
    'echo "list-ready: transient error" >&2; exit 1'

  local waterfall_args_log="$TEST_TMP/waterfall-args.log"
  _install_stub "autonomous-waterfall.sh" \
    "printf '%s\n' \"\$*\" >> '$waterfall_args_log'; printf '{\"tier\":1,\"action\":\"run-backlog\",\"reason\":\"test\"}\n'"

  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  export AUTOSPEC_RUN_CMD="true"

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

  [ "$status" -eq 0 ]
  [ -f "$waterfall_args_log" ]
  # The helper failed -> the loop must NOT have injected --backlog-count at all
  # (neither 0 nor any value) so the waterfall keeps its naive-gh fallback.
  ! grep -q -- '--backlog-count' "$waterfall_args_log"
}

@test "conductor: asks list-ready-issues for remaining worker-cap batch size" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'

  local list_ready_args_log="$TEST_TMP/list-ready-args.log"
  _install_stub "list-ready-issues.sh" \
    "printf '%s\n' \"\$*\" >> '$list_ready_args_log'; printf '{\"ready\":[{\"number\":1},{\"number\":2},{\"number\":3}],\"blocked\":[],\"claimed\":[{\"number\":9}],\"conflicts\":[],\"worker_cap\":{\"max_repo_workers\":3,\"active_count\":1,\"remaining\":2,\"reached\":false},\"batch\":[{\"number\":1},{\"number\":2}]}\n'"

  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; main-health) printf "DECISION:continue\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  export AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=3
  export AUTOSPEC_RUN_CMD="true"

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

  [ "$status" -eq 0 ]
  grep -q -- '--batch-size 3' "$list_ready_args_log"
}

@test "conductor: autospec config overrides env for queue batch request" {
  _install_stub "autonomous-control-channel.sh" 'exit 0'

  mkdir -p "$TEST_TMP/.autospec"
  cat > "$TEST_TMP/.autospec/autospec.yml" <<'YAML'
version: 1
autonomous:
  concurrency:
    batch_size: 2
    max_concurrent_repo_workers: 4
YAML

  local list_ready_args_log="$TEST_TMP/list-ready-config-args.log"
  _install_stub "list-ready-issues.sh" \
    "printf '%s\n' \"\$*\" >> '$list_ready_args_log'; printf '{\"ready\":[{\"number\":1},{\"number\":2},{\"number\":3},{\"number\":4}],\"blocked\":[],\"claimed\":[],\"conflicts\":[],\"worker_cap\":{\"max_repo_workers\":4,\"active_count\":0,\"remaining\":4,\"reached\":false},\"batch\":[{\"number\":1},{\"number\":2},{\"number\":3},{\"number\":4}]}\n'"

  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; main-health) printf "DECISION:continue\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
  export AUTOSPEC_CONFIG_FILE="$TEST_TMP/.autospec/autospec.yml"
  export AUTOSPEC_BATCH_SIZE=1
  export AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=1
  export AUTOSPEC_RUN_CMD="true"

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

  [ "$status" -eq 0 ]
  grep -q -- '--batch-size 4' "$list_ready_args_log"
}
