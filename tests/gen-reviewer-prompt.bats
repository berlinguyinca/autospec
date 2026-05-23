#!/usr/bin/env bats
# tests/gen-reviewer-prompt.bats — bats coverage for gen-reviewer-prompt.sh

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
BIN="$REPO_ROOT/scripts/gen-reviewer-prompt.sh"
FIX="$REPO_ROOT/tests/fixtures/gen-reviewer-prompt"

# Case 1: prefix is present — output contains the cache boundary or bundle fallback
@test "prefix-present: output contains cache boundary marker or bundle fallback text" {
  run bash "$BIN" \
    --pr-diff "$FIX/pr-303.diff" \
    --prev-findings "$FIX/findings-empty.json"
  [ "$status" -eq 0 ]
  echo "$output" | grep -qE "CACHE BOUNDARY|bundle-static-context" \
    || echo "$output" | grep -qE "static prefix skipped|Fused guardian"
}

# Case 2: diff clipping — diffs over 8000 lines are clipped with the marker
@test "diff-clipping: diff over 8000 lines is clipped with marker" {
  large_diff="$(mktemp)"
  python3 -c "
import sys
for i in range(8100):
    sys.stdout.write('+line %d\n' % i)
" > "$large_diff"
  run bash "$BIN" --pr-diff "$large_diff"
  rm -f "$large_diff"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "\[diff clipped at 8000 lines\]"
}

# Case 3: lock-step parity — SKILL.md Phase 4 review step invokes gen-reviewer-prompt.sh
@test "lockstep-parity: SKILL.md Phase 4 review step invokes gen-reviewer-prompt.sh" {
  run grep -q "gen-reviewer-prompt.sh" "$REPO_ROOT/skills/autospec-run/SKILL.md"
  [ "$status" -eq 0 ]
}

# Case 4: missing --pr-diff exits non-zero with MISSING_ARG on stderr
@test "missing --pr-diff: exits non-zero with MISSING_ARG on stderr" {
  run bash "$BIN"
  [ "$status" -ne 0 ]
  echo "${output}${stderr:-}" | grep -q "MISSING_ARG" \
    || printf '%s\n' "${lines[@]}" | grep -q "MISSING_ARG"
}

# Case 5: nonexistent --pr-diff file exits non-zero
@test "nonexistent --pr-diff: exits non-zero" {
  run bash "$BIN" --pr-diff /tmp/does-not-exist-pr.diff
  [ "$status" -ne 0 ]
}

# Case 6: script contains zero claude/codex/gemini CLI invocations
@test "script contains zero claude/codex/gemini CLI invocations" {
  run grep -E "\bclaude\b|\bcodex\b|\bgemini\b" "$BIN"
  [ "$status" -ne 0 ]
}

# Case 7: output contains fused guardian+LGTM rubric
@test "dynamic-suffix: output contains fused guardian+LGTM rubric" {
  run bash "$BIN" \
    --pr-diff "$FIX/pr-303.diff" \
    --prev-findings "$FIX/findings-empty.json"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "LGTM"
}

# Case 8: previous findings appear in output when provided
@test "dynamic-suffix: previous findings appear in output when provided" {
  run bash "$BIN" \
    --pr-diff "$FIX/pr-303.diff" \
    --prev-findings "$FIX/findings-example.txt"
  [ "$status" -eq 0 ]
  echo "$output" | grep -q "DUPLICATE_CODE"
}
