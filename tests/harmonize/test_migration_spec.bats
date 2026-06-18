#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/skills/autospec-harmonize/scripts/gen-migration-spec.mjs"
  FIXTURE_TOKENS="$BATS_TEST_DIRNAME/fixtures/migration-variants.json"
  FIXTURE_INVENTORY="$BATS_TEST_DIRNAME/fixtures/migration-inventory.md"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-migration-spec-XXXXXX)"
  SPEC_PATH="$TEST_TMPDIR/docs/specs/2026-06-16-harmonize-fleet-gui-design.md"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "gen-migration-spec exits 0" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  [ "$status" -eq 0 ]
}

@test "gen-migration-spec writes spec at expected path" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  [ -f "$SPEC_PATH" ]
}

@test "gen-migration-spec spec contains at least one checkbox" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  COUNT=$(grep -c '^- \[ \]' "$SPEC_PATH")
  [ "$COUNT" -ge 1 ]
}

@test "gen-migration-spec has one checkbox per diverging group (>=8 groups from 5 components + 3 inconsistencies)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  COUNT=$(grep -c '^- \[ \]' "$SPEC_PATH")
  [ "$COUNT" -ge 8 ]
}

@test "gen-migration-spec spec contains ## UX findings heading" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  grep -q '^## UX findings' "$SPEC_PATH"
}

@test "gen-migration-spec UX findings section is non-empty" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  # At least one inconsistency line should appear after the ## UX findings heading
  FOUND=$(awk '/^## UX findings/{p=1;next} /^## /{p=0} p && /color-drift|spacing-inconsistency|typography-drift/{print}' "$SPEC_PATH" | wc -l)
  [ "$FOUND" -ge 1 ]
}

@test "gen-migration-spec stdout contains /autospec-define handoff line" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  [[ "$output" == *"/autospec-define "* ]]
}

@test "gen-migration-spec handoff line contains spec path" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  [[ "$output" == *"2026-06-16-harmonize-fleet-gui-design.md"* ]]
}

@test "gen-migration-spec spec is non-empty" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  node "$SCRIPT" \
    --variant baseline \
    --tokens "$FIXTURE_TOKENS" \
    --inventory "$FIXTURE_INVENTORY" \
    --date 2026-06-16 \
    --slug fleet-gui \
    --out "$TEST_TMPDIR"
  [ -s "$SPEC_PATH" ]
}
