#!/usr/bin/env bats
# tests/lint-platform-cfg-exposure.bats
#
# Exercises scripts/lint-platform-cfg-exposure.sh (issue #4173, invariant 2):
# it reports the count and location of `not(target_os = "...")` blocks, marks
# the ones gated off the develop-on platform (the code that has no compiler on
# this host), never counts the escaped-quote string literal, and never counts a
# plain `target_os = "linux"` gate (the opposite of exposure).

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  LINT="$REPO_ROOT/scripts/lint-platform-cfg-exposure.sh"
  FIX="$REPO_ROOT/tests/fixtures/lint-platform-cfg-exposure"
}

@test "--help exits 0 and names the rule" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"PLATFORM_CFG_EXPOSURE"* ]]
}

@test "reports a not(target_os) block with file:line and marks it off-develop-on" {
  run bash "$LINT" --develop-on linux "$FIX/pos-mixed.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'pos-mixed.rs:1: not(target_os = "linux") [off develop-on (linux); not compiled on this host]'* ]]
}

@test "counts every occurrence and the develop-on subset in the summary" {
  run bash "$LINT" --develop-on linux "$FIX/pos-mixed.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 3 not(target_os = ...) block(s) in 1 file(s); 2 off develop-on (linux)'* ]]
}

@test "a different platform is reported but NOT marked off-develop-on" {
  run bash "$LINT" --develop-on linux "$FIX/pos-mixed.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'pos-mixed.rs:5: not(target_os = "windows")'* ]]
  # exactly the two linux blocks carry the off-develop-on marker
  [ "$(printf '%s\n' "$output" | grep -c '^PLATFORM_CFG_EXPOSURE:.*off develop-on (linux)' || true)" -eq 2 ]
}

@test "--develop-on windows moves the marker to the windows block" {
  run bash "$LINT" --develop-on windows "$FIX/pos-mixed.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 3 not(target_os = ...) block(s) in 1 file(s); 1 off develop-on (windows)'* ]]
  [[ "$output" == *'pos-mixed.rs:5: not(target_os = "windows") [off develop-on (windows); not compiled on this host]'* ]]
  # no linux block is marked when the develop-on platform is windows
  [ "$(printf '%s\n' "$output" | grep -c '^PLATFORM_CFG_EXPOSURE:.*not(target_os = "linux").*off develop-on' || true)" -eq 0 ]
}

@test "a plain target_os (linux gate) is the opposite of exposure and is not reported" {
  run bash "$LINT" --develop-on linux "$FIX/neg-linux-gate.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 0 not(target_os = ...) block(s) in 0 file(s); 0 off develop-on (linux)'* ]]
}

@test "an escaped-quote string literal is not a cfg block" {
  run bash "$LINT" --develop-on linux "$FIX/neg-string-literal.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 0 not(target_os = ...) block(s) in 0 file(s); 0 off develop-on (linux)'* ]]
}

@test "a file with no cfg attributes reports zero" {
  run bash "$LINT" --develop-on linux "$FIX/empty.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 0 not(target_os = ...) block(s) in 0 file(s); 0 off develop-on (linux)'* ]]
}

@test "directory scan aggregates across files and counts distinct files" {
  run bash "$LINT" --develop-on linux "$FIX/dir"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 3 not(target_os = ...) block(s) in 2 file(s); 2 off develop-on (linux)'* ]]
}

@test "the escaped-quote string literal in the real tree is not reported" {
  run bash "$LINT" --develop-on linux "$REPO_ROOT/crates/autospec-cli/src/commands/claim/tests/heartbeat_classify.rs"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY: 0 not(target_os = ...) block(s) in 0 file(s); 0 off develop-on (linux)'* ]]
}

@test "default scan of crates/ is internally consistent, non-empty, and exits 0" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
  [[ "$output" == *'PLATFORM_CFG_EXPOSURE_SUMMARY:'* ]]
  # the portable runtime (the file at the centre of #4173) is always flagged
  [[ "$output" == *'/portable_runtime.rs'* ]]
  # the summary count equals the number of finding lines, and is positive
  local nfindings nsummary
  nfindings=$(printf '%s\n' "$output" | grep -c '^PLATFORM_CFG_EXPOSURE:')
  nsummary=$(printf '%s\n' "$output" | grep -oE 'PLATFORM_CFG_EXPOSURE_SUMMARY: [0-9]+' | grep -oE '[0-9]+')
  [ "$nfindings" -eq "$nsummary" ]
  [ "$nsummary" -gt 0 ]
}
