#!/usr/bin/env bats
# upgrade-engine.bats — TDD suite for upgrade-engine.sh (issue #1180)
# No network access, no real installs. All subprocess calls mocked via $TMP/bin PATH shim.

ENGINE="${BATS_TEST_DIRNAME}/../scripts/upgrade-engine.sh"
FX="${BATS_TEST_DIRNAME}/fixtures/engine"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/ue-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/ue-mock-bin.XXXXXX)"
  TAG_LOG="$TEST_ROOT/git-tags.log"
  COMMIT_LOG="$TEST_ROOT/git-commits.log"
  CODEMOD_LOG="$TEST_ROOT/codemod.log"
  BUILD_LOG="$TEST_ROOT/build.log"
  BLOCK_LOG="$TEST_ROOT/behavior-lock.log"
  touch "$TAG_LOG" "$COMMIT_LOG" "$CODEMOD_LOG" "$BUILD_LOG" "$BLOCK_LOG"
  export TEST_ROOT MOCK_BIN TAG_LOG COMMIT_LOG CODEMOD_LOG BUILD_LOG BLOCK_LOG
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Mock helpers ──────────────────────────────────────────────────────────────

# install_git_mock: fake git using a quoted heredoc so all $ are literal in the
# written script, and EXISTING_TAGS is read from the environment at runtime.
install_git_mock() {
  cat > "$MOCK_BIN/git" <<'GITEOF'
#!/usr/bin/env bash
TAG_LOG="${TAG_LOG:-/dev/null}"
COMMIT_LOG="${COMMIT_LOG:-/dev/null}"
EXISTING_TAGS="${EXISTING_TAGS:-}"
case "$1" in
  tag)
    if [ "$2" = "-l" ]; then
      pattern="${3:-}"
      for t in $EXISTING_TAGS; do
        if [ "$t" = "$pattern" ]; then
          printf '%s\n' "$t"
        fi
      done
      exit 0
    fi
    printf '%s\n' "$3" >> "$TAG_LOG"
    exit 0
    ;;
  commit)
    printf '%s\n' "$*" >> "$COMMIT_LOG"
    exit 0
    ;;
  diff|add|status)
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
GITEOF
  chmod +x "$MOCK_BIN/git"
}

# install_codemod_mock: write with quoted heredoc then patch the exit code.
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

# install_build_mock: npm mock that fails the first BUILD_FAIL_COUNT calls.
# Uses a quoted heredoc so all $ are literal in the written script.
install_build_mock() {
  cat > "$MOCK_BIN/npm" <<'NPMEOF'
#!/usr/bin/env bash
BUILD_LOG="${BUILD_LOG:-/dev/null}"
BUILD_FAIL_COUNT="${BUILD_FAIL_COUNT:-0}"
CALL_FILE="${TEST_ROOT}/npm-call-count"
count=0
if [ -f "$CALL_FILE" ]; then count=$(cat "$CALL_FILE"); fi
count=$(( count + 1 ))
printf '%s' "$count" > "$CALL_FILE"
printf 'npm %s (call %s)\n' "$*" "$count" >> "$BUILD_LOG"
if [ "$BUILD_FAIL_COUNT" -gt 0 ] && [ "$count" -le "$BUILD_FAIL_COUNT" ]; then
  printf 'mock build FAIL\n' >&2
  exit 1
fi
exit 0
NPMEOF
  chmod +x "$MOCK_BIN/npm"
}

# install_tsc_mock: quoted heredoc + sed patch.
install_tsc_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/tsc" <<'TSCEOF'
#!/usr/bin/env bash
exit __EC__
TSCEOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/tsc"
  rm -f "$MOCK_BIN/tsc.bak"
  chmod +x "$MOCK_BIN/tsc"
}

# install_behavior_lock_mock: quoted heredoc + sed patch.
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

install_all_mocks() {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
}

# ── Existence ─────────────────────────────────────────────────────────────────

@test "upgrade-engine.sh exists and is executable" {
  [ -x "$ENGINE" ]
}

# ── Argument validation ───────────────────────────────────────────────────────

@test "missing --hops exits non-zero" {
  run bash "$ENGINE"
  [ "$status" -ne 0 ]
}

@test "--hops with non-existent file exits non-zero" {
  run bash "$ENGINE" --hops /nonexistent/hops.json
  [ "$status" -ne 0 ]
}

@test "empty hops list exits 0 with no tags or commits" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-empty.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ ! -s "$TAG_LOG" ]
}

# ── GREEN hop: full success pipeline ─────────────────────────────────────────

@test "green hop: exits 0 on successful single hop" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
}

@test "green hop: pre-upgrade tag is set" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qF 'pre-upgrade-angular-21' "$TAG_LOG"
}

@test "green hop: post-upgrade tag is set after success" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qF 'post-upgrade-angular-21' "$TAG_LOG"
}

@test "green hop: a commit is recorded" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ -s "$COMMIT_LOG" ]
}

@test "green hop: codemod is invoked with framework and target version" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    CODEMOD_LOG="$CODEMOD_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -q 'angular' "$CODEMOD_LOG"
  grep -q '21' "$CODEMOD_LOG"
}

@test "green hop: behavior-lock re-verify is invoked" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BLOCK_LOG="$BLOCK_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ -s "$BLOCK_LOG" ]
}

@test "green multi-hop: all post-upgrade tags set and commits recorded" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-multi.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qF 'post-upgrade-angular-21' "$TAG_LOG"
  grep -qF 'post-upgrade-angular-22' "$TAG_LOG"
  [ "$(wc -l < "$COMMIT_LOG")" -ge 2 ]
}

# ── FAIL-PAST-BOUND: bounded fix-loop ────────────────────────────────────────

@test "fail-past-bound: exits non-zero when all fix attempts exhausted" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BUILD_FAIL_COUNT=999 TEST_ROOT="$TEST_ROOT" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT" --max-fix 3
  [ "$status" -ne 0 ]
}

@test "fail-past-bound: post-upgrade tag NOT set for the failing hop" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BUILD_FAIL_COUNT=999 TEST_ROOT="$TEST_ROOT" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT" --max-fix 3
  ! grep -qF 'post-upgrade-angular-21' "$TAG_LOG"
}

@test "fail-past-bound: pre-upgrade tag remains intact" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BUILD_FAIL_COUNT=999 TEST_ROOT="$TEST_ROOT" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT" --max-fix 3
  grep -qF 'pre-upgrade-angular-21' "$TAG_LOG"
}

@test "fail-past-bound: fix attempts do not exceed max-fix bound" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BUILD_FAIL_COUNT=999 TEST_ROOT="$TEST_ROOT" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT" --max-fix 3
  call_count=0
  if [ -f "$TEST_ROOT/npm-call-count" ]; then
    call_count="$(cat "$TEST_ROOT/npm-call-count")"
  fi
  [ "$call_count" -le 3 ]
}

@test "fail-past-bound: second hop is not started when first fails" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    BUILD_FAIL_COUNT=999 TEST_ROOT="$TEST_ROOT" \
    bash "$ENGINE" --hops "$FX/hops-multi.json" --root "$TEST_ROOT" --max-fix 2
  ! grep -qF 'pre-upgrade-angular-22' "$TAG_LOG"
  ! grep -qF 'post-upgrade-angular-22' "$TAG_LOG"
}

# ── IDEMPOTENCY ───────────────────────────────────────────────────────────────

@test "idempotency: codemod not invoked for a hop whose post-upgrade tag already exists" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    CODEMOD_LOG="$CODEMOD_LOG" \
    EXISTING_TAGS="post-upgrade-angular-21" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ ! -s "$CODEMOD_LOG" ]
}

@test "idempotency: no new tag written for already-completed hop" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    EXISTING_TAGS="post-upgrade-angular-21" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  [ ! -s "$TAG_LOG" ]
}

@test "idempotency: completed first hop is skipped; remaining second hop runs" {
  install_git_mock
  install_codemod_mock 0
  install_build_mock
  install_tsc_mock 0
  install_behavior_lock_mock 0
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    CODEMOD_LOG="$CODEMOD_LOG" \
    EXISTING_TAGS="post-upgrade-angular-21" \
    bash "$ENGINE" --hops "$FX/hops-multi.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qF 'post-upgrade-angular-22' "$TAG_LOG"
  ! grep -qF 'post-upgrade-angular-21' "$TAG_LOG"
}

# ── TAG format ────────────────────────────────────────────────────────────────

@test "tag format: pre-upgrade-<fw>-<to>" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qE '^pre-upgrade-angular-21$' "$TAG_LOG"
}

@test "tag format: post-upgrade-<fw>-<to>" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    TAG_LOG="$TAG_LOG" COMMIT_LOG="$COMMIT_LOG" \
    bash "$ENGINE" --hops "$FX/hops-single.json" --root "$TEST_ROOT"
  [ "$status" -eq 0 ]
  grep -qE '^post-upgrade-angular-21$' "$TAG_LOG"
}
