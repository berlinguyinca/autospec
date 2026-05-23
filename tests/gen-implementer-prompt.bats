#!/usr/bin/env bats
# tests/gen-implementer-prompt.bats — bats coverage for gen-implementer-prompt.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/gen-implementer-prompt.sh"
FIX="$REPO_ROOT/tests/fixtures/gen-implementer-prompt"

# Case 1: output contains "begin coding now" (dynamic suffix verification)
@test "suffix-substitution: output contains 'begin coding now'" {
  run bash "$BIN" \
    --issue-body "$FIX/issue-438.md" \
    --branch feat/example-hello
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "begin coding now"
}

# Case 2: output contains branch name in suffix
@test "suffix-substitution: output contains branch name" {
  run bash "$BIN" \
    --issue-body "$FIX/issue-438.md" \
    --branch feat/example-hello
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "feat/example-hello"
}

# Case 3: output contains issue body content
@test "suffix-substitution: output contains issue body text" {
  run bash "$BIN" \
    --issue-body "$FIX/issue-438.md" \
    --branch feat/example-hello
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "hello world"
}

# Case 4: missing --issue-body exits non-zero with MISSING_ARG on stderr
@test "missing --issue-body: exits non-zero with MISSING_ARG on stderr" {
  run bash "$BIN" --branch feat/example-hello
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "MISSING_ARG" || echo "${lines[@]}" | grep -q "MISSING_ARG"
}

# Case 5: missing --branch exits non-zero
@test "missing --branch: exits non-zero with MISSING_ARG on stderr" {
  run bash "$BIN" --issue-body "$FIX/issue-438.md"
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "MISSING_ARG" || echo "${lines[@]}" | grep -q "MISSING_ARG"
}

# Case 6: script contains zero LLM CLI invocations
@test "script contains zero claude/codex/gemini CLI invocations" {
  run grep -E "\bclaude\b|\bcodex\b|\bgemini\b" "$BIN"
  [ "$status" -ne 0 ]
}
