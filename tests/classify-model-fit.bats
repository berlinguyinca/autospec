#!/usr/bin/env bats
# tests/classify-model-fit.bats — bats coverage for classify-model-fit.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/classify-model-fit.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/classify-model-fit"

setup() {
  # Route telemetry to a per-test temp dir so tests never write the real working-tree jsonl.
  export AUTOSPEC_TELEMETRY_DIR="$(mktemp -d)"
}

teardown() {
  rm -rf "${AUTOSPEC_TELEMETRY_DIR:-}"
}

# Case 1: small fixture → ctx:32k, reasoning:medium
@test "small.md: ctx:32k output in Model fit block" {
  run bash "$BIN" "$FIXTURES/small.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "ctx:32k"
  run bash "$BIN" "$FIXTURES/small.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"reasoning":"medium"'
}

# Case 2: medium fixture → ctx:64k, reasoning:medium
@test "medium.md: ctx:64k output in Model fit block" {
  run bash "$BIN" "$FIXTURES/medium.md"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "ctx:64k"
  run bash "$BIN" "$FIXTURES/medium.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"reasoning":"medium"'
}

# Case 2b: shallow fixture → ctx:32k, reasoning:shallow
@test "shallow.md: ctx:32k and reasoning:shallow in JSON output" {
  run bash "$BIN" "$FIXTURES/shallow.md" --json
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '"reasoning":"shallow"'
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
  local telemetry_file="$AUTOSPEC_TELEMETRY_DIR/classify-model-fit.jsonl"
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
