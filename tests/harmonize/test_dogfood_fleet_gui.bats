#!/usr/bin/env bats
# tests/harmonize/test_dogfood_fleet_gui.bats
#
# Dogfood test: run the harmonize --source-only pipeline against the real
# skills/autospec-fleet/gui directory (inline-styled single-file HTML GUI).
# Asserts exit 0 and that discovered-tokens.json has a non-empty .inconsistencies
# array — proving that extract-source.mjs can read <style> blocks in HTML files.
#
# Guards: node and jq must be available; LLM and Playwright are stubbed out.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TARGET_GUI="$REPO_ROOT/skills/autospec-fleet/gui"
  SCRIPTS_DIR="$REPO_ROOT/skills/autospec-harmonize/scripts"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-dogfood-fleet-gui-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "dogfood fleet-gui: harmonize --source-only exits 0 and produces discovered-tokens.json" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  run env \
    AUTOSPEC_SCRIPTS_DIR="$SCRIPTS_DIR" \
    AUTOSPEC_HARMONIZE_LLM_STUB=1 \
    AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    bash "$SCRIPTS_DIR/harmonize.sh" \
      --root "$TARGET_GUI" \
      --source-only \
      --no-live-preview \
      --variants minimal \
      --out "$TEST_TMPDIR"

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/discovered-tokens.json" ]
}

@test "dogfood fleet-gui: discovered-tokens.json has non-empty inconsistencies array" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  env \
    AUTOSPEC_SCRIPTS_DIR="$SCRIPTS_DIR" \
    AUTOSPEC_HARMONIZE_LLM_STUB=1 \
    AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    bash "$SCRIPTS_DIR/harmonize.sh" \
      --root "$TARGET_GUI" \
      --source-only \
      --no-live-preview \
      --variants minimal \
      --out "$TEST_TMPDIR"

  run jq '.inconsistencies | length >= 1' "$TEST_TMPDIR/discovered-tokens.json"
  [ "$status" -eq 0 ]
  [ "$output" = "true" ]
}
