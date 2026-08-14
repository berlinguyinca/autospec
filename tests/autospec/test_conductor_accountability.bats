#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP"
  FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
  mkdir -p "$FAKE_SCRIPTS" "$HOME/.autospec"
  export AUTOSPEC_QUEUE_BIN="$FAKE_SCRIPTS/autospec"
}

teardown() {
  rm -rf "$TEST_TMP" 2>/dev/null || true
}

install_stub() {
  printf '#!/usr/bin/env bash\n%s\n' "$2" > "$FAKE_SCRIPTS/$1"
  chmod +x "$FAKE_SCRIPTS/$1"
}

@test "conductor: journal failure blocks the selected mutation boundary" {
  install_stub "autonomous-control-channel.sh" 'exit 0'
  install_stub "autonomous-waterfall.sh" 'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
  install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
  install_stub "autonomous-spend-ledger.sh" 'case "${1:-}" in check) printf "continue\n";; esac'
  install_stub "autonomous-resilience.sh" 'case "${1:-}" in state) exit 0;; lock) printf "DECISION:lock-acquired\n";; esac'
  install_stub "autospec" 'if [ "${1:-}" = autonomous ] && [ "${2:-}" = accountability-event ]; then exit 19; fi; printf '\''{"ready":[{"number":42}],"batch":[],"blocked":[],"worker_cap":{"reached":false}}\n'\'''
  mkdir -p "$HOME/.autospec/autonomous-operator/test-owner_test-repo"
  printf '%s\n' '{"accountability":{"run_id":"abc"}}' > "$HOME/.autospec/autonomous-operator/test-owner_test-repo/launch.json"
  export AUTOSPEC_AUTONOMOUS_OPERATOR_DIR="$HOME/.autospec/autonomous-operator"

  run bash -c ". '$LOOP_LIB'; CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' CONDUCTOR_REPO='test-owner/test-repo' CONDUCTOR_MAX_CYCLES=1 CONDUCTOR_POLL_INTERVAL=0 CONDUCTOR_NO_DIGEST=1 autospec_conductor_run" 2>&1

  [[ "$output" == *"accountability selection event journal failed"* ]]
}

@test "conductor: a bound run fails closed when its accountability binary disappears" {
  run bash -c ". '$LOOP_LIB'; AUTOSPEC_ACCOUNTABILITY_REQUIRED=1 _AUTOSPEC_CONDUCTOR_REPO='test-owner/test-repo' _AUTOSPEC_CONDUCTOR_ACCOUNTABILITY_BIN='$TEST_TMP/missing-autospec' _autospec_conductor_accountability_event stopped what why evidence 1" 2>&1

  [ "$status" -ne 0 ]
  [[ "$output" == *"accountability binary is unavailable"* ]]
}
