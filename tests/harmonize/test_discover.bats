#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  DISCOVER="$REPO_ROOT/skills/autospec-harmonize/scripts/design-discover.sh"
  FIXTURE_DIR="$REPO_ROOT/skills/autospec-harmonize/test-targets/messy-app"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-discover-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "no --url: writes discovered-tokens.json with source==source and inventory.md" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq  >/dev/null 2>&1 || skip "jq not available"

  run bash "$DISCOVER" --root "$FIXTURE_DIR" --out "$TEST_TMPDIR"
  [ "$status" -eq 0 ]

  [ -f "$TEST_TMPDIR/discovered-tokens.json" ]
  [ -f "$TEST_TMPDIR/inventory.md" ]

  run jq -r '.source' "$TEST_TMPDIR/discovered-tokens.json"
  [ "$status" -eq 0 ]
  [ "$output" = "source" ]
}

@test "no --url: inventory.md names at least one inconsistency category" {
  command -v node >/dev/null 2>&1 || skip "node not available"

  run bash "$DISCOVER" --root "$FIXTURE_DIR" --out "$TEST_TMPDIR"
  [ "$status" -eq 0 ]

  [ -f "$TEST_TMPDIR/inventory.md" ]
  # inventory.md must contain at least one category heading/label
  run grep -iE "palette|type_scale|components|inconsistenc" "$TEST_TMPDIR/inventory.md"
  [ "$status" -eq 0 ]
}

@test "unreachable --url falls back to source, exits 0, prints harmonize_runtime_unavailable" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq  >/dev/null 2>&1 || skip "jq not available"

  # Use combined output (stdout+stderr) via run's default capture
  run bash "$DISCOVER" --root "$FIXTURE_DIR" --url "http://127.0.0.1:1" --out "$TEST_TMPDIR" 2>&1
  [ "$status" -eq 0 ]

  # Output must mention the unavailability signal (assert deterministically:
  # a mid-body pipeline is not a reliable bats failure point on its own).
  if ! echo "$output" | grep -q "harmonize_runtime_unavailable"; then
    echo "FAIL: expected harmonize_runtime_unavailable in output: $output" >&2
    return 1
  fi

  # Fallback must produce source-extracted profile
  [ -f "$TEST_TMPDIR/discovered-tokens.json" ]
  run jq -r '.source' "$TEST_TMPDIR/discovered-tokens.json"
  [ "$status" -eq 0 ]
  [ "$output" = "source" ]
}
