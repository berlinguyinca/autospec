#!/usr/bin/env bats
# best-practice-migrate.bats — TDD suite for best-practice-migrate.sh (issue #1181)
# No network access, no real installs. All subprocess calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/best-practice-migrate.sh"
FX="${BATS_TEST_DIRNAME}/fixtures/best-practice"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/bp-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/bp-mock-bin.XXXXXX)"
  CODEMOD_LOG="$TEST_ROOT/codemod.log"
  BLOCK_LOG="$TEST_ROOT/behavior-lock.log"
  touch "$CODEMOD_LOG" "$BLOCK_LOG"
  export TEST_ROOT MOCK_BIN CODEMOD_LOG BLOCK_LOG
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Mock helpers ──────────────────────────────────────────────────────────────

install_mock() {
  local name="$1"
  local body="$2"
  local path="$MOCK_BIN/$name"
  printf '#!/usr/bin/env bash\n%s\n' "$body" > "$path"
  chmod +x "$path"
}

# install_git_mock: writes a git mock using a quoted heredoc (all $ literal in
# the written script). EXISTING_TAGS env var controls what tags are visible.
install_git_mock() {
  cat > "$MOCK_BIN/git" <<'GITEOF'
#!/usr/bin/env bash
EXISTING_TAGS="${EXISTING_TAGS:-}"
case "$1" in
  tag)
    if [ "$2" = "-l" ]; then
      pattern="${3:-}"
      for t in $EXISTING_TAGS; do
        case "$t" in
          $pattern) printf '%s\n' "$t" ;;
        esac
      done
      exit 0
    fi
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
GITEOF
  chmod +x "$MOCK_BIN/git"
}

# install_codemod_mock: writes a codemod-route.sh mock that logs invocations.
install_codemod_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/codemod-route.sh" <<'CMEOF'
#!/usr/bin/env bash
CODEMOD_LOG="${CODEMOD_LOG:-/dev/null}"
printf '%s\n' "$*" >> "$CODEMOD_LOG"
exit __EC__
CMEOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/codemod-route.sh"
  rm -f "$MOCK_BIN/codemod-route.sh.bak"
  chmod +x "$MOCK_BIN/codemod-route.sh"
}

# install_behavior_lock_mock: writes a behavior-lock.sh mock that logs calls.
install_behavior_lock_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/behavior-lock.sh" <<'BLEOF'
#!/usr/bin/env bash
BLOCK_LOG="${BLOCK_LOG:-/dev/null}"
printf '%s\n' "$*" >> "$BLOCK_LOG"
exit __EC__
BLEOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/behavior-lock.sh"
  rm -f "$MOCK_BIN/behavior-lock.sh.bak"
  chmod +x "$MOCK_BIN/behavior-lock.sh"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "best-practice-migrate.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── GREEN-GATE: REFUSE when version loop is not green ─────────────────────────
# The script queries git tag -l for post-upgrade-<fw>-<ver> tags.
# If none found (EXISTING_TAGS empty) → exit non-zero, no codemod invoked.

@test "refuse: exit non-zero when no post-upgrade tags exist" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "refuse: codemod recorder empty when no post-upgrade tags" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="" env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT" || true

  # codemod-route.sh must NOT have been invoked
  [ ! -s "$CODEMOD_LOG" ]
}

@test "refuse: output contains versions_not_green message" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"
  printf '%s\n' "$output" | grep -q 'versions_not_green'
}

@test "refuse: --versions-green flag bypasses tag check" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  # With --versions-green the green gate is satisfied regardless of tags
  EXISTING_TAGS="" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT" --versions-green
  [ "$status" -eq 0 ]
}

# ── GATED STEP: green → schematic invoked AND behavior-lock re-verified ────────

@test "gated-step: angular standalone schematic invoked when green (via tag)" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  # Provide a matching post-upgrade tag so the green gate passes
  EXISTING_TAGS="post-upgrade-angular-17" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -q 'angular' "$CODEMOD_LOG"
}

@test "gated-step: behavior-lock re-verify called after schematic (angular)" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-angular-17" env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"

  # behavior-lock.sh must have been called at least once
  [ -s "$BLOCK_LOG" ]
}

@test "gated-step: next async-request-api schematic invoked when green" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-next-14" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-next.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -q 'next' "$CODEMOD_LOG"
}

@test "gated-step: behavior-lock re-verify called after schematic (next)" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-next-14" env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-next.json" --root "$TEST_ROOT"

  [ -s "$BLOCK_LOG" ]
}

@test "gated-step: react types schematic invoked when green" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-react-19" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-react.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -q 'react' "$CODEMOD_LOG"
}

@test "gated-step: behavior-lock re-verify called after schematic (react)" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-react-19" env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-react.json" --root "$TEST_ROOT"

  [ -s "$BLOCK_LOG" ]
}

@test "gated-step: step rejected when behavior-lock re-verify fails" {
  install_git_mock
  install_codemod_mock 0
  # behavior-lock fails → step must not be accepted (non-zero exit)
  install_behavior_lock_mock 1

  EXISTING_TAGS="post-upgrade-angular-17" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

# ── MANUAL MIGRATION: surfaced as follow-up, never attempted ──────────────────
# Manual structural migrations (e.g. Pages→App Router restructure) must appear
# in output as "follow-up:" lines and must NOT produce a codemod invocation.

@test "manual-migration: follow-up line surfaced for angular signals" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-angular-17" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT"
  printf '%s\n' "$output" | grep -qi 'follow-up'
}

@test "manual-migration: follow-up line surfaced for next pages-to-app-router" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-next-14" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-next.json" --root "$TEST_ROOT"
  printf '%s\n' "$output" | grep -qi 'follow-up'
}

@test "manual-migration: follow-up line surfaced for react 19 manual patterns" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-react-19" run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-react.json" --root "$TEST_ROOT"
  printf '%s\n' "$output" | grep -qi 'follow-up'
}

# ── Argument validation ───────────────────────────────────────────────────────

@test "exits non-zero when --detect file is missing" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "/nonexistent/detect.json" --root "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

@test "exits non-zero when --detect argument omitted" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --root "$TEST_ROOT"
  [ "$status" -ne 0 ]
}

# ── follow-up list file written ───────────────────────────────────────────────

@test "follow-up list file written under .autospec/ when green" {
  install_git_mock
  install_codemod_mock 0
  install_behavior_lock_mock 0

  EXISTING_TAGS="post-upgrade-angular-17" env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$FX/detect-angular.json" --root "$TEST_ROOT" \
    --out "$TEST_ROOT/.autospec"

  [ -f "$TEST_ROOT/.autospec/best-practice-followups.txt" ]
}
