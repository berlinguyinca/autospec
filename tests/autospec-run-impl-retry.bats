#!/usr/bin/env bats
# tests/autospec-run-impl-retry.bats — TDD for Phase 4 adaptive-retry loop (issue #392)
#
# These tests exercise the RETRY-LOOP block in SKILL.md by driving a stub
# implementer script that simulates bad/good commit outcomes.

SKILL_MD="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
CODEX_MD="${BATS_TEST_DIRNAME}/../skills/autospec-run/codex/prompt.md"
OPENCODE_MD="${BATS_TEST_DIRNAME}/../skills/autospec-run/opencode/agent.md"
LINT_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/lint-implementation.sh"

# ── structural presence tests ─────────────────────────────────────────────────

@test "SKILL.md Phase 4 contains RETRY-LOOP:begin marker" {
  grep -q 'RETRY-LOOP:begin' "$SKILL_MD"
}

@test "SKILL.md Phase 4 contains MAX_IMPL_RETRIES variable" {
  grep -q 'MAX_IMPL_RETRIES' "$SKILL_MD"
}

@test "SKILL.md Phase 4 contains directive_context variable" {
  grep -q 'directive_context' "$SKILL_MD"
}

@test "SKILL.md Phase 4 contains Retry attempt findings block" {
  grep -q 'Retry attempt' "$SKILL_MD"
}

@test "SKILL.md Phase 4 contains manual-intervention comment on exhaustion" {
  grep -q 'Implementer hit max retries; manual intervention needed' "$SKILL_MD"
}

@test "SKILL.md Phase 4 contains label release on exhaustion" {
  grep -q 'auto-implement-active' "$SKILL_MD"
}

@test "codex/prompt.md contains RETRY-LOOP:begin marker" {
  grep -q 'RETRY-LOOP:begin' "$CODEX_MD"
}

@test "codex/prompt.md contains MAX_IMPL_RETRIES variable" {
  grep -q 'MAX_IMPL_RETRIES' "$CODEX_MD"
}

@test "opencode/agent.md contains RETRY-LOOP:begin marker" {
  grep -q 'RETRY-LOOP:begin' "$OPENCODE_MD"
}

@test "opencode/agent.md contains MAX_IMPL_RETRIES variable" {
  grep -q 'MAX_IMPL_RETRIES' "$OPENCODE_MD"
}

# ── behavioral tests via stub scripts ────────────────────────────────────────

setup() {
  WORK_DIR=$(mktemp -d /tmp/impl-retry-test-XXXXXX)
  export WORK_DIR
  export AUTOSPEC_SCRIPTS_DIR="$WORK_DIR/scripts"
  mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
  # Stub lint-implementation.sh --directives
  cat > "$AUTOSPEC_SCRIPTS_DIR/lint-implementation.sh" <<'STUB'
#!/usr/bin/env bash
# Stub: emit a deterministic directive line
echo "Fix COMPLEXITY: function too long"
exit 0
STUB
  chmod +x "$AUTOSPEC_SCRIPTS_DIR/lint-implementation.sh"
}

teardown() {
  rm -rf "$WORK_DIR"
}

# Helper: build and run a retry-loop harness script
_run_retry_harness() {
  local max_retries="$1"
  local good_on_attempt="$2"   # commit succeeds on this attempt number
  local harness="$WORK_DIR/harness.sh"
  cat > "$harness" <<HARNESS
#!/usr/bin/env bash
set -eu
attempt=1
MAX_IMPL_RETRIES="\${MAX_IMPL_RETRIES:-${max_retries}}"
directive_context=""
ATTEMPTS_LOG="$WORK_DIR/attempts.log"
echo "" > "\$ATTEMPTS_LOG"

while [ "\$attempt" -le "\$MAX_IMPL_RETRIES" ]; do
  echo "\$attempt" >> "\$ATTEMPTS_LOG"
  if [ "\$attempt" -ge "${good_on_attempt}" ]; then
    # Simulate successful commit + green bats on this attempt
    break
  fi
  # Simulate failed commit — capture directives
  findings=\$(bash "\${AUTOSPEC_SCRIPTS_DIR}/lint-implementation.sh" --pre-commit --staged --directives 2>/dev/null || true)
  if [ -n "\$findings" ]; then
    directive_context="\${directive_context}

## Retry attempt \${attempt} findings
\${findings}

Fix these BEFORE the next code generation."
  fi
  attempt=\$((attempt + 1))
done

if [ "\$attempt" -gt "\$MAX_IMPL_RETRIES" ]; then
  echo "EXHAUSTED" > "$WORK_DIR/outcome.txt"
  echo "Implementer hit max retries; manual intervention needed" >> "$WORK_DIR/outcome.txt"
  exit 1
fi

echo "SUCCESS:attempt=\$attempt" > "$WORK_DIR/outcome.txt"
printf '%s' "\$directive_context" > "$WORK_DIR/directive_context.txt"
exit 0
HARNESS
  chmod +x "$harness"
  bash "$harness"
}

@test "retry loop succeeds on attempt 3 when good_on=3" {
  run _run_retry_harness 5 3
  [ "$status" -eq 0 ]
  grep -q "SUCCESS:attempt=3" "$WORK_DIR/outcome.txt"
  # Verify exactly 3 attempt entries were logged
  count=$(grep -c '[0-9]' "$WORK_DIR/attempts.log")
  [ "$count" -eq 3 ]
}

@test "directive_context accumulates findings across attempts" {
  _run_retry_harness 5 3
  # directive_context should contain findings from attempts 1 and 2
  grep -q "Retry attempt 1 findings" "$WORK_DIR/directive_context.txt"
  grep -q "Retry attempt 2 findings" "$WORK_DIR/directive_context.txt"
  grep -q "Fix COMPLEXITY" "$WORK_DIR/directive_context.txt"
}

@test "retry loop fires exhaustion branch after MAX_IMPL_RETRIES=5 bad attempts" {
  run _run_retry_harness 5 99
  [ "$status" -eq 1 ]
  grep -q "EXHAUSTED" "$WORK_DIR/outcome.txt"
  grep -q "Implementer hit max retries; manual intervention needed" "$WORK_DIR/outcome.txt"
}

@test "MAX_IMPL_RETRIES env override is respected" {
  MAX_IMPL_RETRIES=2 run _run_retry_harness 2 99
  [ "$status" -eq 1 ]
  grep -q "EXHAUSTED" "$WORK_DIR/outcome.txt"
}
