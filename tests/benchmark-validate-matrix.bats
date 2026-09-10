#!/usr/bin/env bats
# tests/benchmark-validate-matrix.bats — `autospec benchmark validate-matrix`
# CLI coverage for issue #3328 (provider-neutral benchmark matrix contract).

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/debug/autospec"

setup_file() {
  (cd "$REPO_ROOT" && cargo build -q -p autospec-cli)
}

setup() {
  TEST_TMP="$(mktemp -d)"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "validate-matrix exits 0 on the committed qwen38 fixture and names the winner" {
  run "$BIN" benchmark validate-matrix "$REPO_ROOT/fixtures/qwen38-matrix.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *"winner=q4-k-m-mlx"* ]]
  [[ "$output" == *"faster=true"* ]]
}

@test "validate-matrix preserves skip reasons for unsupported cells" {
  run "$BIN" benchmark validate-matrix "$REPO_ROOT/fixtures/qwen38-matrix.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *"skip: q2-k-trtllm (quantization Q2_K is outside the Q3-Q8 range)"* ]]
}

@test "validate-matrix exits non-zero when a speculative row omits draft token counts" {
  cat > "$TEST_TMP/bad.json" <<'JSON'
{"baseline":{"quantization":"Q4_K_M","runtime":"llama.cpp","node":"n",
  "profile":"p","median_success_seconds":100},
 "rows":[{"candidate_id":"spec","quantization":"Q4_K_M","runtime":"mlx",
  "node":"n","profile":"p","success":true,"success_seconds":80,
  "speculative":true,"draft_tokens":4}]}
JSON
  run "$BIN" benchmark validate-matrix "$TEST_TMP/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"speculative row must record draft_tokens and accepted_draft_tokens"* ]]
}

@test "validate-matrix exits non-zero when a row lacks a profile" {
  cat > "$TEST_TMP/blank.json" <<'JSON'
{"baseline":{"quantization":"Q4_K_M","runtime":"llama.cpp","node":"n",
  "profile":"p","median_success_seconds":100},
 "rows":[{"candidate_id":"blank","quantization":"Q4_K_M","runtime":"mlx",
  "node":"n","profile":"","success":false}]}
JSON
  run "$BIN" benchmark validate-matrix "$TEST_TMP/blank.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"blank: profile must be identified"* ]]
}

@test "validate-matrix --json emits a machine-readable report" {
  run "$BIN" benchmark validate-matrix "$REPO_ROOT/fixtures/qwen38-matrix.json" --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"valid":true'* ]]
  [[ "$output" == *'"winner":"q4-k-m-mlx"'* ]]
  [[ "$output" == *'"faster_than_baseline":true'* ]]
}

@test "validate-matrix with no arguments prints usage and exits non-zero" {
  run "$BIN" benchmark validate-matrix
  [ "$status" -ne 0 ]
  [[ "$output" == *"usage: autospec benchmark validate-matrix"* ]]
}
