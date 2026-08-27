#!/usr/bin/env bats
# tests/gen-pr-report.bats — bats coverage for gen-pr-report.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/gen-pr-report.sh"
FIX="$REPO_ROOT/tests/fixtures/gen-pr-report"

# Case 1: green-clean — all tests pass, no drift
@test "green-clean: output matches golden expected-green.md" {
  run bash "$BIN" \
    --gate "$FIX/gate-green.json" \
    --drift "$FIX/drift-empty.json" \
    --loop-log "$FIX/loop.log" \
    --mode test
  [ "$status" -eq 0 ]
  # Must start with marker
  echo "$output" | head -1 | grep -q "<!-- autospec-test-report-marker -->"
  # Must contain passed status
  echo "$output" | grep -q "✅ passed"
  # Must contain mode
  echo "$output" | grep -q "Mode.*test"
  # Must contain iter info
  echo "$output" | grep -q "Iterations:.*2 / 5"
  # Must not contain LLM CLI names
  echo "$output" | grep -qv "claude\|codex\|gemini" || true
}

# Case 2: behind-rebased — branch needs rebase
@test "behind-rebased: output matches golden expected-behind-rebased.md" {
  run bash "$BIN" \
    --gate "$FIX/gate-behind.json" \
    --drift "$FIX/drift-empty.json" \
    --mode test
  [ "$status" -eq 0 ]
  echo "$output" | head -1 | grep -q "<!-- autospec-test-report-marker -->"
  echo "$output" | grep -q "needs rebase"
  echo "$output" | grep -q "branch is 3 commits behind main"
}

# Case 3: max-iters-hit — hit iteration limit
@test "max-iters-hit: output matches golden expected-max-iters.md" {
  run bash "$BIN" \
    --gate "$FIX/gate-max-iters.json" \
    --drift "$FIX/drift-empty.json" \
    --mode test
  [ "$status" -eq 0 ]
  echo "$output" | head -1 | grep -q "<!-- autospec-test-report-marker -->"
  echo "$output" | grep -q "max iterations"
  echo "$output" | grep -q "Iterations:.*5 / 5"
  echo "$output" | grep -q "tests still failing"
}

# Case 4: drift-only — drift detected
@test "drift-only: output matches golden expected-drift-only.md" {
  run bash "$BIN" \
    --gate "$FIX/gate-drift.json" \
    --drift "$FIX/drift-findings.json" \
    --mode test
  [ "$status" -eq 0 ]
  echo "$output" | head -1 | grep -q "<!-- autospec-test-report-marker -->"
  echo "$output" | grep -q "drift"
  echo "$output" | grep -q "API_REFERENCE.md"
}

# Case 5: missing --gate exits non-zero with MISSING_INPUT:gate on stderr
@test "missing --gate: exits non-zero with MISSING_INPUT:gate on stderr" {
  run bash "$BIN" --mode test
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "MISSING_INPUT:gate" || echo "${lines[@]}" | grep -q "MISSING_INPUT:gate"
}

# Case 6: zero LLM invocations in script source
@test "script contains zero claude/codex/gemini CLI invocations" {
  # Verify no LLM CLI calls in the script itself
  run grep -E "\bclaude\b|\bcodex\b|\bgemini\b" "$BIN"
  [ "$status" -ne 0 ]  # grep exits 1 when no match found
}

# Case 7: jq alternative-operator coercion guard — literal drift "passed":false
# must render the Doc drift block even when the gate file's own outcome/metrics
# say nothing about drift. This exercises DRIFT_PASSED="$(jq -r '.passed // true' ...)"
# in isolation from the gate-drift.json metrics-array path used by case 4.
@test "drift passed:false (literal) renders Doc drift block" {
  run bash "$BIN" \
    --gate "$FIX/gate-plain-pass.json" \
    --drift "$FIX/drift-explicit-false.json" \
    --mode test
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "### Doc drift"
  echo "$output" | grep -q "COERCION_TEST.md"
}

# Case 8: absent "passed" key in the drift file must still default to true
# (no drift block), proving the fix did not change the deliberate absent-key
# default — only a literal false must trip the block.
@test "drift file with absent passed key defaults to no drift block" {
  run bash "$BIN" \
    --gate "$FIX/gate-plain-pass.json" \
    --drift "$FIX/drift-absent-key.json" \
    --mode test
  [ "$status" -eq 0 ]
  drift_hits="$(printf '%s' "$output" | grep -c "### Doc drift" || true)"
  [ "${drift_hits:-0}" -eq 0 ]
  doc_hits="$(printf '%s' "$output" | grep -c "SHOULD_NOT_APPEAR.md" || true)"
  [ "${doc_hits:-0}" -eq 0 ]
}
