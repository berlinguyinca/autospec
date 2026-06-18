#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  EXTRACTOR="$REPO_ROOT/skills/autospec-harmonize/scripts/extract-source.mjs"
  FIXTURE_DIR="$REPO_ROOT/skills/autospec-harmonize/test-targets/messy-app"
  PROFILE_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-token-profile.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-extract-source-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "extract-source exits 0 on fixture dir" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run node "$EXTRACTOR" --root "$FIXTURE_DIR"
  [ "$status" -eq 0 ]
}

@test "extract-source output validates against token-profile schema" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v ajv  >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  run ajv validate -s "$PROFILE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
}

@test "extract-source output has source == \"source\"" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  run jq -r '.source' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
  [ "$output" = "source" ]
}

@test "extract-source palette contains at least 3 entries" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  run jq '.palette | length' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
  [ "$output" -ge 3 ]
}

@test "extract-source palette contains the 3 near-duplicate blues from fixture" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  # Each blue must be present in the palette
  run jq -e '[.palette[].hex] | map(ascii_downcase) | contains(["#1a73e8"])' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]

  run jq -e '[.palette[].hex] | map(ascii_downcase) | contains(["#1b74e9"])' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]

  run jq -e '[.palette[].hex] | map(ascii_downcase) | contains(["#1a72e7"])' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
}

@test "extract-source inconsistencies has at least 1 entry" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  run jq '.inconsistencies | length' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}

@test "extract-source inconsistencies entries have a category field" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"

  run jq -e '.inconsistencies | map(has("category")) | all' "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]
}
