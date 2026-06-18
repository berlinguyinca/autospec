#!/usr/bin/env bats
# migration-doc.bats — TDD suite for migration-doc.sh (issue #1183)
# Mocks autospec-doc via $TMP/bin PATH shim. Uses real jq. No network.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/migration-doc.sh"
FIXTURE_DIR="${BATS_TEST_DIRNAME}/fixtures/migration-doc"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/md-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/md-mock-bin.XXXXXX)"
  AUTOSPEC_DOC_LOG="$MOCK_BIN/autospec-doc-calls.txt"
  touch "$AUTOSPEC_DOC_LOG"
  export TEST_ROOT MOCK_BIN AUTOSPEC_DOC_LOG

  # Install autospec-doc mock that records invocations
  cat > "$MOCK_BIN/autospec-doc" <<'MOCKEOF'
#!/usr/bin/env bash
# Mock autospec-doc — records all args to $AUTOSPEC_DOC_LOG
printf '%s\n' "$*" >> "$AUTOSPEC_DOC_LOG"
exit 0
MOCKEOF
  chmod +x "$MOCK_BIN/autospec-doc"
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Helpers ───────────────────────────────────────────────────────────────────

run_migration_doc() {
  run env PATH="$MOCK_BIN:$PATH" "$SCRIPT" "$@"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "migration-doc.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── One section per hop ───────────────────────────────────────────────────────

@test "generates one ## heading per hop in the report" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  [ -n "$doc_file" ]
  local heading_count
  heading_count="$(grep -c '^## ' "$doc_file")"
  [ "$heading_count" -eq 2 ]
}

@test "heading for first hop mentions angular and version range 20 to 21" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -qi 'angular' "$doc_file"
  grep -q '20' "$doc_file"
  grep -q '21' "$doc_file"
}

@test "heading for second hop mentions version range 21 to 22" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -q '22' "$doc_file"
}

# ── Required fields in each section ──────────────────────────────────────────

@test "document contains codemods label" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -qi 'codemod' "$doc_file"
}

@test "document lists the first codemod value from fixture" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -q 'ng update @angular/core@21' "$doc_file"
}

@test "document contains manual_fixes label" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -qi 'manual' "$doc_file"
}

@test "document lists a manual_fix value from fixture" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -q 'NgModule' "$doc_file"
}

@test "document contains residual_risk label" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -qi 'residual' "$doc_file"
}

@test "document lists the residual_risk value from first hop" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -q 'Lazy-loaded modules' "$doc_file"
}

@test "document contains before/after version information label" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  grep -qi 'before\|from\|version' "$doc_file"
}

# ── autospec-doc composition asserted via mock ────────────────────────────────

@test "composes autospec-doc (mock was invoked)" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ -s "$AUTOSPEC_DOC_LOG" ]
}

@test "composes autospec-doc with --full flag" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -q -- '--full' "$AUTOSPEC_DOC_LOG"
}

# ── Output file location ──────────────────────────────────────────────────────

@test "writes markdown doc under --out directory" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_count
  doc_count="$(find "$TEST_ROOT" -name '*.md' | wc -l | tr -d ' ')"
  [ "$doc_count" -ge 1 ]
}

@test "doc path is printed to stdout" {
  run_migration_doc \
    --report "$FIXTURE_DIR/upgrade-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '\.md'
}

# ── Error handling ────────────────────────────────────────────────────────────

@test "exits non-zero when --report file is missing" {
  run_migration_doc \
    --report "$TEST_ROOT/nonexistent-report.json" \
    --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "exits non-zero when --report is not valid JSON" {
  local bad_json="$TEST_ROOT/bad.json"
  printf 'not json at all\n' > "$bad_json"
  run_migration_doc \
    --report "$bad_json" \
    --out "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "default report path is .autospec/upgrade-report.json under root when no --report given" {
  mkdir -p "$TEST_ROOT/.autospec"
  cp "$FIXTURE_DIR/upgrade-report.json" "$TEST_ROOT/.autospec/upgrade-report.json"
  run_migration_doc \
    --root "$TEST_ROOT" \
    --out "$TEST_ROOT"
  [ "$status" -eq 0 ]
  local doc_file
  doc_file="$(find "$TEST_ROOT" -name '*.md' | head -1)"
  [ -n "$doc_file" ]
}
