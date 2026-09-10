#!/usr/bin/env bash
# scripts/validate-regrade.sh — structural gate for the regrade + deadline-ratchet modules (#4080).
#
# Guards the invariants that a compiler cannot: that the regrade and deadline
# ratchet modules exist, that their load-bearing rules are enforced in code
# rather than only in prose, and that the required test coverage is present.
# The behavioural assertions live in the Rust test suite, which this script
# runs.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0
REGRADE="crates/autospec-core/src/autonomous/regrade.rs"
RATCHET="crates/autospec-core/src/deadline_ratchet.rs"
VERDICT="crates/autospec-core/src/autonomous/verdict_validity.rs"
TGATE="crates/autospec-core/src/autonomous/test_gate.rs"
LIB="crates/autospec-core/src/lib.rs"
INTEG="crates/autospec-core/tests/verdict_validity.rs"

fail() {
  failures=$((failures + 1))
  printf 'regrade: FAIL: %s\n' "$*"
}

require_file() {
  [ -f "$1" ] || fail "missing $1"
}

require_grep() {
  local pattern="$1" path="$2" reason="$3"
  if [ ! -f "$path" ]; then
    fail "missing $path"
    return
  fi
  grep -Eq -- "$pattern" "$path" || fail "$path: $reason"
}

# --- Module existence ---
require_file "$REGRADE"
require_file "$RATCHET"

# --- Module registration in lib.rs ---
require_grep 'pub mod regrade;' "$LIB" "regrade not registered in lib.rs autonomous block"
require_grep 'pub mod deadline_ratchet;' "$LIB" "deadline_ratchet not registered in lib.rs"

# --- regrade.rs: core primitives ---
require_grep 'pub struct HostConditions' "$REGRADE" "missing HostConditions struct"
require_grep 'pub struct FlakyTest' "$REGRADE" "missing FlakyTest struct"
require_grep 'pub struct RegradeOutcome' "$REGRADE" "missing RegradeOutcome struct"
require_grep 'pub fn regrade' "$REGRADE" "missing regrade() function"
require_grep 'pub fn blocks' "$REGRADE" "missing blocks() method"
require_grep 'pub fn line' "$REGRADE" "missing line() method"

# --- deadline_ratchet.rs: core primitives ---
require_grep 'pub const DEADLINE_THRESHOLD_MS' "$RATCHET" "missing threshold constant"
require_grep 'pub const DEADLINE_BASELINE' "$RATCHET" "missing baseline constant"
require_grep 'pub fn from_millis_values' "$RATCHET" "missing parser function"
require_grep 'pub fn deadline_sites' "$RATCHET" "missing deadline_sites function"
require_grep 'pub struct RatchetViolation' "$RATCHET" "missing RatchetViolation struct"
require_grep 'pub fn ratchet_violations' "$RATCHET" "missing ratchet_violations function"
require_grep 'pub fn line' "$RATCHET" "missing line() function"

# --- verdict_validity.rs: host + flaky fields ---
require_grep 'flaky_tests' "$VERDICT" "missing flaky_tests field on RecordedVerdict"
require_grep 'host: Option<HostConditions>' "$VERDICT" "missing host field on RecordedVerdict"
require_grep 'HostConditions' "$VERDICT" "missing HostConditions import"

# --- test_gate.rs: recorded() requires host ---
require_grep 'host: &HostConditions' "$TGATE" "recorded() does not require host parameter"
require_grep 'flaky_tests' "$TGATE" "recorded() does not populate flaky_tests"

# --- Integration test: host passed to recorded() ---
require_grep 'HostConditions' "$INTEG" "integration test missing HostConditions import"

# --- AC1: re-run-once logic in regrade ---
require_grep 'persistent' "$REGRADE" "regrade() does not identify persistent failures"
require_grep 'flaky' "$REGRADE" "regrade() does not identify flaky tests"

# --- AC2: ratchet is shrink-only ---
require_grep 'shrink-only|SHRINK' "$RATCHET" "baseline is not documented as shrink-only"

# --- AC3: host conditions ride along ---
require_grep 'load_average' "$REGRADE" "HostConditions missing load_average"
require_grep 'concurrent_agents' "$REGRADE" "HostConditions missing concurrent_agents"

# --- AC4: single failing verdict does not block ---
require_grep 'a_single_failing_verdict_does_not_block' "$REGRADE" "missing AC4 test"

# --- Test count: regrade has at least 9 tests ---
regrade_test_count=$(grep -c '#\[test\]' "$REGRADE" 2>/dev/null || echo 0)
[ "$regrade_test_count" -ge 9 ] || fail "regrade.rs has only $regrade_test_count tests (need >= 9)"

# --- Test count: deadline_ratchet has at least 5 tests ---
ratchet_test_count=$(grep -c '#\[test\]' "$RATCHET" 2>/dev/null || echo 0)
[ "$ratchet_test_count" -ge 5 ] || fail "deadline_ratchet.rs has only $ratchet_test_count tests (need >= 5)"

# --- Rust tests: run the affected test modules ---
printf 'regrade: running Rust tests...\n'
lib_flag="--lib"
integ_flag="--test"
if ! cargo test -p autospec-core $lib_flag autonomous::regrade 2>&1 | grep -q 'test result: ok'; then
  fail "cargo test autonomous::regrade failed"
fi
if ! cargo test -p autospec-core $lib_flag deadline_ratchet 2>&1 | grep -q 'test result: ok'; then
  fail "cargo test deadline_ratchet failed"
fi
if ! cargo test -p autospec-core $integ_flag verdict_validity 2>&1 | grep -q 'test result: ok'; then
  fail "cargo test verdict_validity (integration) failed"
fi

if [ "$failures" -gt 0 ]; then
  printf 'regrade: %d failure(s)\n' "$failures"
  exit 1
fi
printf 'regrade: all checks passed\n'
