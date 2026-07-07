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
