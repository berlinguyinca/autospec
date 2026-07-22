#!/usr/bin/env bats
ROOT="$BATS_TEST_DIRNAME/../../.."
FIXTURE="$BATS_TEST_DIRNAME/fixtures/screenshot-audit.json"
@test "guardrail generator selects risky routes and omits count assertions by default" {
  run node "$ROOT/skills/autospec-qa/scripts/gen-layout-guardrails.mjs" --input "$FIXTURE"
  [ "$status" -eq 0 ]; [ "$(grep -c 'dashboard' <<< "$output")" -ge 1 ];
  ! grep -q 'Primitive counts are opt-in' <<< "$output"
  grep -q 'component-level scroll containers are allowed' <<< "$output"
}
@test "strict counts enables explicit count assertion" {
  run node "$ROOT/skills/autospec-qa/scripts/gen-layout-guardrails.mjs" --input "$FIXTURE" --strict-counts
  [ "$status" -eq 0 ]; grep -q 'Primitive counts are opt-in' <<< "$output"
}
@test "documentation generator emits route audit fields" {
  run node "$ROOT/skills/autospec-doc/scripts/gen-ui-audit-doc.mjs" --input "$FIXTURE"
  [ "$status" -eq 0 ]; grep -q 'Document overflow' <<< "$output"; grep -q 'Primitive counts' <<< "$output"
}
