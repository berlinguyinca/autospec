#!/usr/bin/env bats
# tests/classify-model-fit.bats — bats coverage for classify-model-fit.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/classify-model-fit.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/classify-model-fit"

setup() {
  # Use a temp telemetry dir so tests don't pollute the real one
  export TELEMETRY_TEST_DIR="$(mktemp -d)"
  # Point the script to a temp repo root by overriding via env trick:
  # The script uses REPO_ROOT derived from SCRIPT_DIR; we cannot override it
  # directly, but we can at least ensure .autospec/telemetry/ is created
  # under the worktree (which is fine for CI).
  :
}

teardown() {
  rm -rf "${TELEMETRY_TEST_DIR:-}"
}

# Case 1: small fixture → ctx:32k
@test "small.md: ctx:32k output in Model fit block" {
  run bash "$BIN" "$FIXTURES/small.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "ctx:32k"
}

# Case 2: medium fixture → ctx:64k
@test "medium.md: ctx:64k output in Model fit block" {
  run bash "$BIN" "$FIXTURES/medium.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "ctx:64k"
}

# Case 3: large fixture (8+ files, cross-skill) → ctx:120k
@test "large.md: ctx:120k output in Model fit block" {
  run bash "$BIN" "$FIXTURES/large.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "ctx:120k"
}

# Case 4: deep-reasoning fixture → reasoning:deep
@test "deep-reasoning.md: reasoning:deep output in Model fit block" {
  run bash "$BIN" "$FIXTURES/deep-reasoning.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "reasoning:deep"
}

# Case 5: low-confidence escalation stub — LLM_ESCALATION_THRESHOLD=1.0 forces LLM path
# Since omc may not be available, we expect either graceful fallback (exit 0) or exit 2.
# We verify deterministic:false appears in JSON mode.
@test "low-confidence: LLM_ESCALATION_THRESHOLD=1.0 invokes LLM path (json mode)" {
  run env LLM_ESCALATION_THRESHOLD=1.0 bash "$BIN" "$FIXTURES/low-confidence.md" --json
  # Exit 0 (LLM fallback succeeded or omc unavailable with graceful default) or 2 (LLM failed)
  [ "$status" -eq 0 ] || [ "$status" -eq 2 ]
  # Output should contain deterministic:false
  echo "$output" | grep -q '"deterministic":false'
}

# Case 6: telemetry-append — one JSON line appended per invocation
@test "telemetry: .autospec/telemetry/classify-model-fit.jsonl gains one line per invocation" {
  local telemetry_file="$REPO_ROOT/.autospec/telemetry/classify-model-fit.jsonl"
  local before=0
  if [ -f "$telemetry_file" ]; then
    before="$(wc -l < "$telemetry_file")"
  fi

  run bash "$BIN" "$FIXTURES/small.md"
  [ "$status" -eq 0 ]

  local after=0
  if [ -f "$telemetry_file" ]; then
    after="$(wc -l < "$telemetry_file")"
  fi

  [ "$after" -gt "$before" ]
}

# Case 7: LLM_ESCALATION_THRESHOLD env override
@test "LLM_ESCALATION_THRESHOLD env var overrides default 0.3" {
  # Setting threshold to 0.0 means confidence is always >= threshold → deterministic path
  run env LLM_ESCALATION_THRESHOLD=0.0 bash "$BIN" "$FIXTURES/small.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"deterministic":true'
}
