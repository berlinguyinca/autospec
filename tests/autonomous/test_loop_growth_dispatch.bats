#!/usr/bin/env bats
# tests/autonomous/test_loop_growth_dispatch.bats
# Coverage for the GROWTH dispatch branches (run-growth-define,
# service-growth-outbound, run-growth-measure) wired into
# autospec_conductor_run() in scripts/lib/autospec-loop.sh.
#
# Mirrors tests/autospec/test_conductor_wiring.bats's harness/stub shape
# (the Tier-3 run-architecture-improvement tests in particular). All gh
# calls, helper scripts, and notify.sh are stubbed via a fake PATH
# directory so no real GitHub calls or desktop notifications are emitted.

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
printf 'notify: %s — %s\n' "${1:-}" "${2:-}" >&2
exit 0
EOF
  chmod +x "$FAKE_BIN/notify.sh"

  export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN
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

# ── run-growth-define ─────────────────────────────────────────────────────
@test "run-growth-define dispatches AUTOSPEC_GROWTH_DEFINE_CMD" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":6,"action":"run-growth-define","reason":"test"}\n'\'''

  local gd_log="$TEST_TMP/growth-define.log"
  export AUTOSPEC_GROWTH_DEFINE_CMD="printf '{\"dry\":false,\"filed\":2}\n'; printf 'growth-define-called\n' >> '$gd_log'"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$gd_log" ]
  grep -q 'growth-define-called' "$gd_log"
  [[ "$output" == *"Tier G1 growth-define result: dry=false filed=2"* ]]
}

@test "run-growth-define failure does not abort the loop" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":6,"action":"run-growth-define","reason":"test"}\n'\'''

  export AUTOSPEC_GROWTH_DEFINE_CMD="exit 1"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"Tier G1 growth-define result: dry=true filed=0"* ]]
  [[ "$output" == *"Tier G1 dry (tierg-dry-cycles=1)"* ]]
}

# ── service-growth-outbound ───────────────────────────────────────────────
@test "service-growth-outbound dispatches AUTOSPEC_GROWTH_OUTBOUND_CMD" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":5,"action":"service-growth-outbound","reason":"test"}\n'\'''

  local go_log="$TEST_TMP/growth-outbound.log"
  export AUTOSPEC_GROWTH_OUTBOUND_CMD="printf '{\"dry\":false,\"filed\":1}\n'; printf 'growth-outbound-called\n' >> '$go_log'"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$go_log" ]
  grep -q 'growth-outbound-called' "$go_log"
  [[ "$output" == *"Tier G2 growth-outbound result: dry=false"* ]]
}

@test "service-growth-outbound failure does not abort the loop" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":5,"action":"service-growth-outbound","reason":"test"}\n'\'''

  export AUTOSPEC_GROWTH_OUTBOUND_CMD="exit 1"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"Tier G2 growth-outbound result: dry=true"* ]]
}

# ── run-growth-measure ────────────────────────────────────────────────────
@test "run-growth-measure dispatches AUTOSPEC_GROWTH_MEASURE_CMD" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":7,"action":"run-growth-measure","reason":"test"}\n'\'''

  local gm_log="$TEST_TMP/growth-measure.log"
  export AUTOSPEC_GROWTH_MEASURE_CMD="printf '{\"dry\":false,\"filed\":0}\n'; printf 'growth-measure-called\n' >> '$gm_log'"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$gm_log" ]
  grep -q 'growth-measure-called' "$gm_log"
  [[ "$output" == *"Tier G3 growth-measure result: dry=false"* ]]
}

@test "run-growth-measure failure does not abort the loop" {
  _install_common_stubs
  _install_stub "autonomous-waterfall.sh" \
    'printf '\''{"tier":7,"action":"run-growth-measure","reason":"test"}\n'\'''

  export AUTOSPEC_GROWTH_MEASURE_CMD="exit 1"

  _run_cycle

  [ "$status" -eq 0 ]
  [[ "$output" == *"Tier G3 growth-measure result: dry=true"* ]]
}

# ── Safety invariant: growth-disabled repos never see growth flags ────────
@test "growth-disabled (no .autospec/growth.yml): waterfall never receives --growth-* flags" {
  _install_common_stubs
  local waterfall_args_log="$TEST_TMP/waterfall-args.log"
  _install_stub "autonomous-waterfall.sh" \
    "printf '%s\n' \"\$*\" >> '$waterfall_args_log'; printf '{\"tier\":1,\"action\":\"run-backlog\",\"reason\":\"test\"}\n'"

  export AUTOSPEC_RUN_CMD="true"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$waterfall_args_log" ]
  ! grep -q -- '--growth-' "$waterfall_args_log"
}

@test "growth-enabled (.autospec/growth.yml present + valid): waterfall receives --growth-enabled 1" {
  _install_common_stubs
  mkdir -p "$TEST_TMP/.autospec"
  cat > "$TEST_TMP/.autospec/growth.yml" <<'YAML'
product:
  name: Acme
site:
  url: https://acme.dev
  repo_path: .
measurement: {}
approval:
  control_repo: acme/growth
YAML

  cp "$REPO_ROOT/skills/autospec-shared/scripts/validate-growth-config.sh" \
    "$FAKE_SCRIPTS/validate-growth-config.sh"
  chmod +x "$FAKE_SCRIPTS/validate-growth-config.sh"

  local waterfall_args_log="$TEST_TMP/waterfall-args.log"
  _install_stub "autonomous-waterfall.sh" \
    "printf '%s\n' \"\$*\" >> '$waterfall_args_log'; printf '{\"tier\":1,\"action\":\"run-backlog\",\"reason\":\"test\"}\n'"

  # growth-measure-due.sh isn't installed in FAKE_SCRIPTS for this test; the
  # loop must tolerate the missing helper (guarded call -> 0/not-due).
  export AUTOSPEC_RUN_CMD="true"

  _run_cycle

  [ "$status" -eq 0 ]
  [ -f "$waterfall_args_log" ]
  grep -q -- '--growth-enabled 1' "$waterfall_args_log"
}
