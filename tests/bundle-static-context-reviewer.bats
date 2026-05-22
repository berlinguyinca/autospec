#!/usr/bin/env bats
# tests/bundle-static-context-reviewer.bats — TDD for --role reviewer in
# skills/autospec-shared/scripts/bundle-static-context.sh (issue #404)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"

@test "bundle-static-context.sh --role reviewer exits 0" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
}

@test "bundle-static-context.sh --role reviewer emits opening CACHE BOUNDARY marker" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | head -1 | grep -q "CACHE BOUNDARY"
}

@test "bundle-static-context.sh --role reviewer emits closing CACHE BOUNDARY marker" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | tail -1 | grep -q "CACHE BOUNDARY"
}

@test "bundle-static-context.sh --role reviewer output contains Implementation-quality contract" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Implementation-quality contract"
}

@test "bundle-static-context.sh --role reviewer output contains RULE_ID table" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "OUT_OF_SCOPE"
}

@test "bundle-static-context.sh --role reviewer output contains reviewer-verdict-format block" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qi "verdict\|LGTM"
}

@test "bundle-static-context.sh --role reviewer output contains lint-implementation.sh schema" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "lint-implementation.sh"
}

@test "bundle-static-context.sh --role reviewer output is idempotent" {
  out1=$("$SCRIPT" --role reviewer)
  out2=$("$SCRIPT" --role reviewer)
  [ "$out1" = "$out2" ]
}
