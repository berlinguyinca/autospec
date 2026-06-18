#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  GENERALIZER="$REPO_ROOT/skills/autospec-harmonize/scripts/design-generalize.mjs"
  FIXTURE_DIR="$REPO_ROOT/skills/autospec-harmonize/test-targets/messy-app"
  EXTRACTOR="$REPO_ROOT/skills/autospec-harmonize/scripts/extract-source.mjs"
  VARIANT_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-variant.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-generalize-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

# Helper: build a profile JSON from the messy-app fixture
_build_profile() {
  node "$EXTRACTOR" --root "$FIXTURE_DIR" > "$TEST_TMPDIR/profile.json"
}

@test "design-generalize exits 0 with stub and fixture profile" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  _build_profile
  run env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json"
  [ "$status" -eq 0 ]
}

@test "design-generalize output id == \"baseline\"" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  run jq -r '.id' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" = "baseline" ]
}

@test "design-generalize output axis == \"baseline\"" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  run jq -r '.axis' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" = "baseline" ]
}

@test "design-generalize palette has exactly 1 role:primary entry" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  run jq '[.tokens.palette[] | select(.role=="primary")] | length' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "design-generalize output validates against variant schema" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v ajv  >/dev/null 2>&1 || skip "ajv CLI not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  run ajv validate -s "$VARIANT_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
}

@test "design-generalize output has non-empty label and design_md" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  run jq -r '.label | length' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" -gt 0 ]
  run jq -r '.design_md | length' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" -gt 0 ]
}

@test "design-generalize tokens.palette items have hex field matching pattern" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  _build_profile
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/profile.json" \
    > "$TEST_TMPDIR/variant.json"
  # All hex values must match ^#[0-9a-fA-F]{6}$
  run jq -e '[.tokens.palette[].hex | test("^#[0-9a-fA-F]{6}$")] | all' "$TEST_TMPDIR/variant.json"
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]
}

@test "design-generalize inline fixture profile also produces exactly 1 primary" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  # Write an inline messy fixture with 3 near-duplicate blues + other colors
  cat > "$TEST_TMPDIR/inline-profile.json" <<'EOF'
{
  "source": "source",
  "palette": [
    {"hex": "#1a73e8", "count": 10},
    {"hex": "#1b74e9", "count": 8},
    {"hex": "#1a72e7", "count": 6},
    {"hex": "#ffffff", "count": 20},
    {"hex": "#111111", "count": 15},
    {"hex": "#d93025", "count": 3}
  ],
  "type_scale": [{"px": 12}, {"px": 14}, {"px": 16}, {"px": 18}, {"px": 32}],
  "spacing": [{"px": 4}, {"px": 8}, {"px": 10}, {"px": 12}, {"px": 16}, {"px": 24}],
  "radii": [{"px": 2}, {"px": 4}, {"px": 6}, {"px": 8}, {"px": 24}],
  "shadows": [
    {"value": "0 1px 3px rgba(0,0,0,0.2)"},
    {"value": "0 2px 8px rgba(0,0,0,0.15)"},
    {"value": "0 4px 12px rgba(26,114,231,0.4)"}
  ],
  "components": {"button": []},
  "inconsistencies": []
}
EOF
  env AUTOSPEC_HARMONIZE_LLM_STUB=1 node "$GENERALIZER" --tokens "$TEST_TMPDIR/inline-profile.json" \
    > "$TEST_TMPDIR/inline-variant.json"
  run jq '[.tokens.palette[] | select(.role=="primary")] | length' "$TEST_TMPDIR/inline-variant.json"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}
