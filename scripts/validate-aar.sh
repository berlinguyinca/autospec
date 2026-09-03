#!/usr/bin/env bash
# scripts/validate-aar.sh — structural gate for the Adaptive Agent Runtime.
#
# Guards the invariants that a compiler cannot: that every AAR module named by
# the specification exists, that the load-bearing rules are enforced in code
# rather than only in prose, and that the required test coverage is present.
# The behavioural assertions live in the Rust test suite, which this script
# runs single-threaded so a filesystem-touching case cannot race another.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0
CORE="crates/autospec-core/src/aar"
TESTS="crates/autospec-core/tests"

fail() {
  failures=$((failures + 1))
  printf 'aar: FAIL: %s\n' "$*"
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

# Spec sections 3-15: one module per responsibility.
for module in \
  classify/mod.rs classify/rubric.rs context.rs escalation.rs guards.rs \
  inferweave.rs memory.rs mod.rs outcome.rs pi.rs policy.rs profile.rs \
  reasoning.rs telemetry.rs topology.rs; do
  require_file "$CORE/$module"
done

# Spec section 21: the required test coverage.
for suite in \
  aar_classification.rs aar_context_memory.rs aar_e2e_scenarios.rs \
  aar_escalation.rs aar_guards.rs aar_inferweave.rs aar_pi_adapter.rs \
  aar_policy.rs aar_profiles.rs aar_reasoning.rs aar_telemetry_outcome.rs \
  aar_topology.rs; do
  require_file "$TESTS/$suite"
done
require_file "crates/autospec-cli/tests/aar_commands.rs"
require_file "crates/autospec-cli/src/commands/aar.rs"

# Spec section 5: the documented starting budgets.
require_grep 'tiny: 512' "$CORE/reasoning.rs" "tiny budget must default to 512 tokens"
require_grep 'normal: 2_048' "$CORE/reasoning.rs" "normal budget must default to 2048 tokens"
require_grep 'complex: 4_096' "$CORE/reasoning.rs" "complex budget must default to 4096 tokens"
require_grep 'exceptional: 8_192' "$CORE/reasoning.rs" \
  "exceptional budget must default to 8192 tokens"

# Spec section 6: full conversation history is never injected by default.
require_grep 'include_full_history: false' "$CORE/context.rs" \
  "context policy must default to excluding full history"

# Spec section 9: the working rules are pinned verbatim.
require_grep 'When acceptance criteria are satisfied, STOP\.' "$CORE/pi.rs" \
  "the harness working rules must carry the stop rule verbatim"

# Spec section 10: the documented edit guard defaults.
require_grep 'max_edit_lines: 150' "$CORE/guards.rs" "max_edit_lines must default to 150"
require_grep 'max_new_file_lines: 300' "$CORE/guards.rs" \
  "max_new_file_lines must default to 300"

# Spec section 8/13: separation of duties is enforced in code, and re-checked
# on every fallback rather than assumed to survive one.
require_grep 'pub fn enforce_separation' "$CORE/topology.rs" \
  "separation of duties must be enforced programmatically"
require_grep 'preserves_separation_after_fallback' "$CORE/escalation.rs" \
  "escalation must re-check separation of duties on every fallback"

# Spec section 12: free context is a hard filter, not a score contribution.
require_grep 'free_context_tokens < required_free' "$CORE/inferweave.rs" \
  "insufficient free context must reject a node outright"

# Spec section 14: prompt tokens must decompose into cached plus new prefill.
require_grep 'new_prefill_tokens != self.prompt_tokens' "$CORE/telemetry.rs" \
  "telemetry must reject token accounting that does not add up"

# Spec section 7: the durable memory files.
for memory_file in task.md plan.md state.md findings.md decisions.md tests.md review.md; do
  require_grep "\"$memory_file\"" "$CORE/memory.rs" "memory file $memory_file must be declared"
done

if [ "${AAR_SKIP_CARGO:-0}" != "1" ]; then
  # Single-threaded on purpose: the CLI suite writes into worktrees, and this
  # gate must not depend on the order two cases happen to interleave in.
  if ! cargo test -p autospec-core --test aar_classification --test aar_context_memory \
    --test aar_e2e_scenarios --test aar_escalation --test aar_guards --test aar_inferweave \
    --test aar_pi_adapter --test aar_policy --test aar_profiles --test aar_reasoning \
    --test aar_telemetry_outcome --test aar_topology -- --test-threads=1 >/dev/null 2>&1; then
    fail "cargo test for the autospec-core aar suites failed"
  fi
  if ! cargo test -p autospec-cli --test aar_commands -- --test-threads=1 >/dev/null 2>&1; then
    fail "cargo test for the autospec-cli aar command suite failed"
  fi
fi

if [ "$failures" -ne 0 ]; then
  printf 'aar: %d check(s) failed\n' "$failures"
  exit 1
fi

printf 'aar: OK\n'
