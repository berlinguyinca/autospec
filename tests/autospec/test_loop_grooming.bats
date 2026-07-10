#!/usr/bin/env bats
# tests/autospec/test_loop_grooming.bats
# Coverage for the Tier-1.5 grooming telemetry/reconcile/govern block in
# scripts/lib/autospec-loop.sh (autospec-grooming-llm-fill Task 6).
#
# Drives autospec_conductor_run() for one Tier-1.5 promote-open-issues cycle
# with a stub promoter envelope carrying BOTH .promoted[] (eligible promotes)
# and .routed[] groom entries (groom-canary / groom-auto), stubs
# groom-reconcile.sh / grooming-observe.sh / grooming-govern.sh via their
# AUTOSPEC_GROOMING_*_BIN seams, and asserts:
#   - one telemetry line per promoted issue with template_groomed:false;
#   - one line per routed groom with template_groomed:true and correct canary;
#   - groom-reconcile.sh runs BEFORE grooming-observe.sh (ordered marker file);
#   - govern tick receives --min-samples == configured budget.canary_floor.
#
# Reuses the test_conductor_wiring.bats harness (FAKE_SCRIPTS stubs + fake PATH).

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  mkdir -p "$HOME/.autospec"

  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS"
  cp "$REPO_ROOT/scripts/autospec-runtime-config.sh" "$FAKE_SCRIPTS/autospec-runtime-config.sh"

  FAKE_BIN="$TEST_TMP/fake-bin"
  mkdir -p "$FAKE_BIN"
  export PATH="$FAKE_BIN:$PATH"

  cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  issue) echo "[]" ;;
  repo)  echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
  *)     exit 0 ;;
esac
EOF
  chmod +x "$FAKE_BIN/gh"

  cat > "$FAKE_BIN/notify.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$FAKE_BIN/notify.sh"

  # Directory for grooming seam stubs.
  GROOM_BIN="$TEST_TMP/groom-bin"
  mkdir -p "$GROOM_BIN"

  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN GROOM_BIN
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

_install_baseline_stubs() {
  _install_stub "autonomous-control-channel.sh" 'exit 0'
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":1.5,"action":"promote-open-issues","reason":"open issues"}\n'\'''
  _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  _install_stub "autonomous-spend-ledger.sh" \
    'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
  _install_stub "autonomous-resilience.sh" \
    'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
  _install_stub "autospec-usage-limit.sh" 'exit 0'
}

@test "loop grooming: dual telemetry (promoted false / routed true+canary), reconcile-before-observe, canary_floor min-samples" {
  _install_baseline_stubs

  TELE="$TEST_TMP/tele.jsonl"
  ORDER_LOG="$TEST_TMP/order.log"
  GOVERN_LOG="$TEST_TMP/govern.log"
  RECONCILE_LOG="$TEST_TMP/reconcile.log"

  # reconcile stub → records order + full args, exits 0.
  cat > "$GROOM_BIN/groom-reconcile.sh" <<'SH'
#!/usr/bin/env bash
printf 'reconcile\n' >> "$ORDER_LOG"
printf '%s\n' "$*" >> "$RECONCILE_LOG"
exit 0
SH
  chmod +x "$GROOM_BIN/groom-reconcile.sh"

  # observe stub → records order, emits a non-empty envelope so govern ticks.
  cat > "$GROOM_BIN/grooming-observe.sh" <<'SH'
#!/usr/bin/env bash
printf 'observe\n' >> "$ORDER_LOG"
printf '{"groomed_clean_merge_rate":0,"baseline_clean_merge_rate":0,"samples":0,"baseline_samples":0}\n'
exit 0
SH
  chmod +x "$GROOM_BIN/grooming-observe.sh"

  # govern stub → echoes its args to a log.
  cat > "$GROOM_BIN/grooming-govern.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GOVERN_LOG"
printf '{"active":["eligible-promote"]}\n'
exit 0
SH
  chmod +x "$GROOM_BIN/grooming-govern.sh"

  # config stub → policy=auto, budget.canary_floor=7.
  cat > "$GROOM_BIN/grooming-config.sh" <<'SH'
#!/usr/bin/env bash
key=""
while [ $# -gt 0 ]; do case "$1" in --key) key="$2"; shift 2;; *) shift;; esac; done
case "$key" in
  policy) printf 'auto\n' ;;
  budget.canary_floor) printf '7\n' ;;
  *) printf '\n' ;;
esac
SH
  chmod +x "$GROOM_BIN/grooming-config.sh"

  export ORDER_LOG GOVERN_LOG RECONCILE_LOG
  export AUTOSPEC_GROOMING_TELEMETRY="$TELE"
  export AUTOSPEC_GROOMING_RECONCILE_BIN="$GROOM_BIN/groom-reconcile.sh"
  export AUTOSPEC_GROOMING_OBSERVE_BIN="$GROOM_BIN/grooming-observe.sh"
  export AUTOSPEC_GROOMING_GOVERN_BIN="$GROOM_BIN/grooming-govern.sh"
  export AUTOSPEC_GROOMING_CONFIG_BIN="$GROOM_BIN/grooming-config.sh"
  # Deliberately do NOT set AUTOSPEC_GROOMING_MIN_SAMPLES (config must drive it).
  unset AUTOSPEC_GROOMING_MIN_SAMPLES

  # Promoter envelope: two eligible promotes + one canary + one auto groom +
  # one non-groom route (must be ignored by the telemetry emit).
  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD='printf '\''{"dry":false,"filed":4,"promoted":[101,102],"routed":[{"issue":201,"action":"groom-canary"},{"issue":202,"action":"groom-auto"},{"issue":203,"action":"route:split"}]}\n'\'''

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

  # Telemetry file must exist and carry one line per promoted + routed groom.
  [ -f "$TELE" ]

  # Promoted issues → template_groomed:false.
  [ "$(jq -r 'select(.issue==101)|.template_groomed' "$TELE")" = "false" ]
  [ "$(jq -r 'select(.issue==102)|.template_groomed' "$TELE")" = "false" ]

  # Routed grooms → template_groomed:true, canary per action.
  [ "$(jq -r 'select(.issue==201)|.template_groomed' "$TELE")" = "true" ]
  [ "$(jq -r 'select(.issue==201)|.canary' "$TELE")" = "true" ]
  [ "$(jq -r 'select(.issue==202)|.template_groomed' "$TELE")" = "true" ]
  [ "$(jq -r 'select(.issue==202)|.canary' "$TELE")" = "false" ]

  # Non-groom route must NOT be emitted.
  [ -z "$(jq -r 'select(.issue==203)|.issue' "$TELE")" ]

  # All emitted records carry closing_pr:null + outcome:null.
  [ "$(jq -r 'select(.issue==101)|.closing_pr' "$TELE")" = "null" ]
  [ "$(jq -r 'select(.issue==201)|.outcome' "$TELE")" = "null" ]

  # reconcile ran BEFORE observe.
  [ -f "$ORDER_LOG" ]
  rline="$(grep -n '^reconcile$' "$ORDER_LOG" | head -1 | cut -d: -f1)"
  oline="$(grep -n '^observe$' "$ORDER_LOG" | head -1 | cut -d: -f1)"
  [ -n "$rline" ]
  [ -n "$oline" ]
  [ "$rline" -lt "$oline" ]

  # govern tick min-samples == configured canary_floor (7).
  [ -f "$GOVERN_LOG" ]
  grep -q -- '--min-samples 7' "$GOVERN_LOG"

  # reconcile was invoked with its telemetry path + repo flags.
  [ -f "$RECONCILE_LOG" ]
  grep -q -- '--telemetry' "$RECONCILE_LOG"
  grep -q -- '--repo test-owner/test-repo' "$RECONCILE_LOG"
}

@test "loop grooming: reconcile failure (exit 1) does not abort the cycle or skip the govern tick" {
  _install_baseline_stubs

  TELE="$TEST_TMP/tele-fail.jsonl"
  GOVERN_LOG_FAIL="$TEST_TMP/govern-fail.log"

  # reconcile stub → fails; the loop calls it with `|| true` so this must not
  # abort the cycle.
  cat > "$GROOM_BIN/groom-reconcile.sh" <<'SH'
#!/usr/bin/env bash
exit 1
SH
  chmod +x "$GROOM_BIN/groom-reconcile.sh"

  # observe stub → emits a non-empty envelope so govern ticks.
  cat > "$GROOM_BIN/grooming-observe.sh" <<'SH'
#!/usr/bin/env bash
printf '{"groomed_clean_merge_rate":0,"baseline_clean_merge_rate":0,"samples":0,"baseline_samples":0}\n'
exit 0
SH
  chmod +x "$GROOM_BIN/grooming-observe.sh"

  # govern stub → records that it ran.
  cat > "$GROOM_BIN/grooming-govern.sh" <<'SH'
#!/usr/bin/env bash
printf 'govern-ran\n' >> "$GOVERN_LOG_FAIL"
printf '{"active":["eligible-promote"]}\n'
exit 0
SH
  chmod +x "$GROOM_BIN/grooming-govern.sh"

  # config stub → policy=auto, budget.canary_floor=7.
  cat > "$GROOM_BIN/grooming-config.sh" <<'SH'
#!/usr/bin/env bash
key=""
while [ $# -gt 0 ]; do case "$1" in --key) key="$2"; shift 2;; *) shift;; esac; done
case "$key" in
  policy) printf 'auto\n' ;;
  budget.canary_floor) printf '7\n' ;;
  *) printf '\n' ;;
esac
SH
  chmod +x "$GROOM_BIN/grooming-config.sh"

  export GOVERN_LOG_FAIL
  export AUTOSPEC_GROOMING_TELEMETRY="$TELE"
  export AUTOSPEC_GROOMING_RECONCILE_BIN="$GROOM_BIN/groom-reconcile.sh"
  export AUTOSPEC_GROOMING_OBSERVE_BIN="$GROOM_BIN/grooming-observe.sh"
  export AUTOSPEC_GROOMING_GOVERN_BIN="$GROOM_BIN/grooming-govern.sh"
  export AUTOSPEC_GROOMING_CONFIG_BIN="$GROOM_BIN/grooming-config.sh"
  unset AUTOSPEC_GROOMING_MIN_SAMPLES

  export AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD='printf '\''{"dry":false,"filed":4,"promoted":[101,102],"routed":[{"issue":201,"action":"groom-canary"},{"issue":202,"action":"groom-auto"},{"issue":203,"action":"route:split"}]}\n'\'''

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

  # A failed reconcile (called with `|| true`) must not abort the cycle.
  [ "$status" -eq 0 ]

  # The govern tick must still have happened downstream of the failed reconcile.
  [ -f "$GOVERN_LOG_FAIL" ]
  grep -q -- 'govern-ran' "$GOVERN_LOG_FAIL"
}
