#!/usr/bin/env bats
# tests/gen-ac-tests.bats — TDD for scripts/gen-ac-tests.sh (issue #391)

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/gen-ac-tests.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/gen-ac-tests"

setup() {
  mkdir -p "$FIXTURES_DIR"
  # Fixture issue body with AC checkboxes
  cat > "$FIXTURES_DIR/issue-body-simple.md" <<'EOF'
## Acceptance criteria

- [ ] `scripts/gen-ac-tests.sh` is executable and writes a syntactically valid bats file
- [ ] `scripts/gen-ac-tests.sh --verify <dir>` exits 0 with no remaining stubs; exits 1 with stub list otherwise
- [ ] All bats tests pass
EOF

  # Fixture issue body where last AC triggers auto-run assertion
  cat > "$FIXTURES_DIR/issue-body-bats.md" <<'EOF'
## Acceptance criteria

- [ ] bats tests pass for the feature
- [ ] npm test exits 0 on all platforms
EOF

  mkdir -p "$BATS_TMPDIR/ac-clean"
  mkdir -p "$BATS_TMPDIR/ac-stubs"
}

teardown() {
  rm -rf "$FIXTURES_DIR" "$BATS_TMPDIR/ac-clean" "$BATS_TMPDIR/ac-stubs"
}

@test "gen-ac-tests.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "gen-ac-tests.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "gen-ac-tests.sh emits one @test stub per checkbox" {
  run "$SCRIPT" --issue-body "$FIXTURES_DIR/issue-body-simple.md" --out "$BATS_TMPDIR/out-simple.bats"
  [ "$status" -eq 0 ]
  [ -f "$BATS_TMPDIR/out-simple.bats" ]
  count=$(grep -c '@test' "$BATS_TMPDIR/out-simple.bats")
  [ "$count" -eq 3 ]
}

@test "gen-ac-tests.sh output is a valid bats file (shebang present)" {
  run "$SCRIPT" --issue-body "$FIXTURES_DIR/issue-body-simple.md" --out "$BATS_TMPDIR/out-shebang.bats"
  [ "$status" -eq 0 ]
  head -1 "$BATS_TMPDIR/out-shebang.bats" | grep -q 'bats'
}

@test "gen-ac-tests.sh emits skip stub for non-trivial criteria" {
  run "$SCRIPT" --issue-body "$FIXTURES_DIR/issue-body-simple.md" --out "$BATS_TMPDIR/out-skip.bats"
  [ "$status" -eq 0 ]
  grep -q 'skip "auto-stub"' "$BATS_TMPDIR/out-skip.bats"
}

@test "gen-ac-tests.sh emits run assertion when criterion contains 'bats tests'" {
  run "$SCRIPT" --issue-body "$FIXTURES_DIR/issue-body-bats.md" --out "$BATS_TMPDIR/out-bats.bats"
  [ "$status" -eq 0 ]
  grep -q 'run bats' "$BATS_TMPDIR/out-bats.bats"
}

@test "gen-ac-tests.sh --verify exits 0 on clean dir (no stubs)" {
  # Write a clean bats file with no skip auto-stub. printf keeps the @test out of
  # line-start position so bats does not parse the fixture as a real test.
  printf '%s\n' \
    '#!/usr/bin/env bats' \
    '@test "AC#1: something real" {' \
    '  run echo ok' \
    '  [ "$status" -eq 0 ]' \
    '}' > "$BATS_TMPDIR/ac-clean/clean.bats"
  run "$SCRIPT" --verify "$BATS_TMPDIR/ac-clean"
  [ "$status" -eq 0 ]
}

@test "gen-ac-tests.sh --verify exits 1 when stubs present" {
  printf '%s\n' \
    '#!/usr/bin/env bats' \
    '@test "AC#1: criterion" {' \
    '  skip "auto-stub"' \
    '}' > "$BATS_TMPDIR/ac-stubs/stubs.bats"
  run "$SCRIPT" --verify "$BATS_TMPDIR/ac-stubs"
  [ "$status" -eq 1 ]
}

@test "gen-ac-tests.sh --verify prints stub file list on failure" {
  printf '%s\n' \
    '#!/usr/bin/env bats' \
    '@test "AC#1: criterion" {' \
    '  skip "auto-stub"' \
    '}' > "$BATS_TMPDIR/ac-stubs/stubs2.bats"
  run "$SCRIPT" --verify "$BATS_TMPDIR/ac-stubs"
  [ "$status" -eq 1 ]
  echo "$output" | grep -q 'auto-stub'
}

@test "gen-ac-tests.sh --verify exits 0 on empty dir" {
  empty_dir="$BATS_TMPDIR/ac-empty"
  mkdir -p "$empty_dir"
  run "$SCRIPT" --verify "$empty_dir"
  [ "$status" -eq 0 ]
}
