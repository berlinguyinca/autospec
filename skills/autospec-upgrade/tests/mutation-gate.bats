#!/usr/bin/env bats
# mutation-gate.bats — TDD suite for mutation-gate.sh (issue #1175)
# No network access, no real installs. All subprocess calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/mutation-gate.sh"
FIXTURE_DIR="${BATS_TEST_DIRNAME}/fixtures/mutation-gate"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/mg-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/mg-mock-bin.XXXXXX)"
  AUTOSPEC_DIR="$TEST_ROOT/.autospec"
  mkdir -p "$AUTOSPEC_DIR"
  export TEST_ROOT MOCK_BIN AUTOSPEC_DIR
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Helper: install a mock npx that emits a Stryker-style report ──────────────
# MOCK_STRYKER_SCORE env var controls the reported mutation score (default 80).

install_npx_mock() {
  local score="${MOCK_STRYKER_SCORE:-80}"
  cat > "$MOCK_BIN/npx" <<MOCK
#!/usr/bin/env bash
# Fake npx — emits a Stryker-style report when called as "npx stryker run"
if [ "\$1" = "stryker" ] && [ "\$2" = "run" ]; then
  printf 'Ran 100 tests\\n'
  printf 'Mutation score: %s%%\\n' "\${MOCK_STRYKER_SCORE:-${score}}"
  printf 'Killed: 80 Survived: %s Timeout: 0 No coverage: 0\\n' "\$(( 100 - \${MOCK_STRYKER_SCORE:-${score}} ))"
  exit 0
fi
# Pass through anything else
exec "\$@"
MOCK
  chmod +x "$MOCK_BIN/npx"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "mutation-gate.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── --baseline mode: records valid mutation-proof.json ───────────────────────

@test "baseline: exit 0 when Stryker succeeds" {
  install_npx_mock
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

@test "baseline: writes .autospec/mutation-proof.json" {
  install_npx_mock
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=75 \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  [ -f "$proof" ]
}

@test "baseline: mutation-proof.json has numeric baseline.score" {
  install_npx_mock
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=75 \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  [ -f "$proof" ]
  # .baseline.score must be a number (jq -e exits 0 only for truthy non-null)
  jq -e '.baseline.score | type == "number"' "$proof"
}

@test "baseline: mutation-proof.json baseline.score matches mocked Stryker score" {
  install_npx_mock
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=73 \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  local score
  score="$(jq -r '.baseline.score' "$proof")"
  [ "$score" = "73" ]
}

@test "baseline: default out dir is .autospec/ under current directory" {
  install_npx_mock
  # Run from TEST_ROOT without --out; expect .autospec/mutation-proof.json relative to cwd
  local proof="$TEST_ROOT/.autospec/mutation-proof.json"
  mkdir -p "$TEST_ROOT/.autospec"
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    bash -c "cd '$TEST_ROOT' && '$SCRIPT' --baseline"
  [ -f "$proof" ]
}

# ── --gate mode: pass when post-upgrade score >= baseline ────────────────────

@test "gate: exit 0 when post-upgrade score equals baseline" {
  install_npx_mock
  # Write a baseline proof first
  printf '{"baseline":{"score":80}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

@test "gate: exit 0 when post-upgrade score exceeds baseline" {
  install_npx_mock
  printf '{"baseline":{"score":75}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=85 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

# ── --gate mode: fail when post-upgrade score < baseline ─────────────────────

@test "gate: exit non-zero when post-upgrade score is below baseline" {
  install_npx_mock
  # baseline=80, post-upgrade score=70 → should fail
  printf '{"baseline":{"score":80}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=70 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "gate: exit non-zero when post-upgrade score is well below baseline" {
  install_npx_mock
  printf '{"baseline":{"score":90}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=60 \
    "$SCRIPT" --gate 50 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "gate: output mentions surviving mutants when gate fails" {
  install_npx_mock
  printf '{"baseline":{"score":80}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=70 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -qi 'surviv'
}

# ── --gate mode: fail when score is below explicit threshold even if >= baseline

@test "gate: exit non-zero when score below explicit threshold despite >= baseline" {
  install_npx_mock
  # baseline=60, score=65 (>= baseline), but threshold=70 → should fail
  printf '{"baseline":{"score":60}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=65 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

# ── --gate mode: writes post_upgrade score to mutation-proof.json ─────────────

@test "gate: writes post_upgrade.score into mutation-proof.json on pass" {
  install_npx_mock
  printf '{"baseline":{"score":70}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  jq -e '.post_upgrade.score | type == "number"' "$proof"
}

@test "gate: mutation-proof.json contains threshold and passed fields" {
  install_npx_mock
  printf '{"baseline":{"score":70}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  jq -e 'has("threshold") and has("passed")' "$proof"
}

@test "gate: mutation-proof.json passed field is true on pass" {
  install_npx_mock
  printf '{"baseline":{"score":70}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  jq -e '.passed == true' "$proof"
}

@test "gate: mutation-proof.json passed field is false on failure" {
  install_npx_mock
  printf '{"baseline":{"score":80}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=70 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  jq -e '.passed == false' "$proof"
}

# ── runner mapping: jest → jest testRunner ────────────────────────────────────

@test "runner-map: jest detect file maps to jest testRunner in Stryker invocation" {
  # Install a recording npx that captures its arguments
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --runner jest --out "$AUTOSPEC_DIR"

  # The script must invoke npx stryker run with jest config/runner
  grep -q "stryker" "$MOCK_BIN/npx-args.txt"
}

@test "runner-map: vitest detect file maps to vitest testRunner" {
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --runner vitest --out "$AUTOSPEC_DIR"

  grep -q "stryker" "$MOCK_BIN/npx-args.txt"
}

@test "runner-map: karma detect file maps to karma testRunner" {
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --runner karma --out "$AUTOSPEC_DIR"

  grep -q "stryker" "$MOCK_BIN/npx-args.txt"
}

# ── runner mapping: --detect file auto-detects runner ────────────────────────

@test "runner-map: --detect jest fixture produces jest testRunner config" {
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
# Capture all args to a file for inspection
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --detect "$FIXTURE_DIR/detect-jest.json" --out "$AUTOSPEC_DIR"

  # Must have invoked stryker
  grep -q "stryker" "$MOCK_BIN/npx-args.txt"
  # Stryker config file should exist and reference jest
  local cfg="$AUTOSPEC_DIR/stryker.config.json"
  [ -f "$cfg" ]
  jq -e '.testRunner == "jest"' "$cfg"
}

@test "runner-map: --detect vitest fixture produces vitest testRunner config" {
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --detect "$FIXTURE_DIR/detect-vitest.json" --out "$AUTOSPEC_DIR"

  local cfg="$AUTOSPEC_DIR/stryker.config.json"
  [ -f "$cfg" ]
  jq -e '.testRunner == "vitest"' "$cfg"
}

@test "runner-map: --detect karma fixture produces karma testRunner config" {
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$MOCK_BIN/npx-args.txt"
printf 'Mutation score: 80%%\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --detect "$FIXTURE_DIR/detect-karma.json" --out "$AUTOSPEC_DIR"

  local cfg="$AUTOSPEC_DIR/stryker.config.json"
  [ -f "$cfg" ]
  jq -e '.testRunner == "karma"' "$cfg"
}

# ── error: baseline absent when --gate invoked without prior --baseline ───────

@test "gate: exit non-zero when mutation-proof.json has no baseline" {
  install_npx_mock
  # Write a proof file missing the baseline.score field
  printf '{"recorded_at":"2026-06-18"}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "gate: exit non-zero when mutation-proof.json is missing entirely" {
  install_npx_mock
  run env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

# ── error: unparseable Stryker output exits non-zero ─────────────────────────

@test "baseline: exit non-zero when Stryker produces no parseable score" {
  # npx mock that succeeds but emits no score line
  cat > "$MOCK_BIN/npx" <<'MOCK'
#!/usr/bin/env bash
printf 'Stryker ran but something went wrong. No score available.\n'
exit 0
MOCK
  chmod +x "$MOCK_BIN/npx"

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

# ── proof JSON shape contract ─────────────────────────────────────────────────

@test "proof-shape: baseline-only proof has exactly baseline.score (number)" {
  install_npx_mock
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=82 \
    "$SCRIPT" --baseline --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  # Must have baseline.score as a number
  jq -e '.baseline.score | type == "number"' "$proof"
  # Must NOT have post_upgrade yet (only baseline mode was run)
  jq -e 'has("post_upgrade") | not' "$proof"
}

@test "proof-shape: gate proof has baseline.score, post_upgrade.score, threshold, passed" {
  install_npx_mock
  printf '{"baseline":{"score":70}}\n' > "$AUTOSPEC_DIR/mutation-proof.json"
  env PATH="$MOCK_BIN:$PATH" MOCK_STRYKER_SCORE=80 \
    "$SCRIPT" --gate 70 --out "$AUTOSPEC_DIR"
  local proof="$AUTOSPEC_DIR/mutation-proof.json"
  jq -e '
    (.baseline.score | type == "number") and
    (.post_upgrade.score | type == "number") and
    (.threshold | type == "number") and
    (.passed | type == "boolean")
  ' "$proof"
}
