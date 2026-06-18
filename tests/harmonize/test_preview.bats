#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PREVIEW="$REPO_ROOT/skills/autospec-harmonize/scripts/design-preview.mjs"
  # 3-variant fixture (baseline/minimal/high-contrast); the high-contrast label
  # intentionally contains "WCAG" to prove the annotation count is element-based.
  VARIANTS="$BATS_TEST_DIRNAME/fixtures/preview-variants.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-preview-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "design-preview exits 0 with NO_PLAYWRIGHT=1 fallback" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  [ "$status" -eq 0 ]
}

@test "design-preview creates index.html with NO_PLAYWRIGHT=1" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  [ -f "$TEST_TMPDIR/preview/index.html" ]
}

@test "design-preview index.html has exactly 3 data-variant= sections" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  COUNT=$(grep -c 'data-variant=' "$TEST_TMPDIR/preview/index.html")
  [ "$COUNT" -eq 3 ]
}

@test "design-preview index.html has exactly 3 WCAG annotations" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  # Count the annotation ELEMENT, not a bare "WCAG" substring: a realistic
  # variant label (e.g. "High-Contrast — WCAG-AA+") legitimately contains the
  # word WCAG, so substring counting would over-count on real input.
  COUNT=$(grep -c 'class="contrast"' "$TEST_TMPDIR/preview/index.html")
  [ "$COUNT" -eq 3 ]
}

@test "design-preview prints code_health:harmonize_preview_no_render with NO_PLAYWRIGHT=1" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run bash -c "AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 node '$PREVIEW' \
    --variants '$VARIANTS' --out '$TEST_TMPDIR' 2>&1"
  [ "$status" -eq 0 ]
  [[ "$output" == *"harmonize_preview_no_render"* ]]
}

@test "design-preview index.html contains each variant id" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  grep -q 'baseline' "$TEST_TMPDIR/preview/index.html"
  grep -q 'minimal' "$TEST_TMPDIR/preview/index.html"
  grep -q 'high-contrast' "$TEST_TMPDIR/preview/index.html"
}

@test "design-preview index.html is non-empty" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$VARIANTS" --out "$TEST_TMPDIR"
  [ -s "$TEST_TMPDIR/preview/index.html" ]
}
