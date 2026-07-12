#!/usr/bin/env bats
# behavior-lock.bats — TDD suite for behavior-lock.sh (issue #1174)
# No network access, no real installs. All subprocess calls mocked via $TMP/bin PATH shim.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/behavior-lock.sh"

# ── Helper: set up a minimal project root with a package.json ────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/bl-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/bl-mock-bin.XXXXXX)"

  # Create a minimal package.json so upgrade-detect.sh can run (optional—
  # behavior-lock.sh works without it; we pass --detect or --root directly)
  cat > "$TEST_ROOT/package.json" <<'PKGJSON'
{
  "dependencies": { "react": "^18.2.0" },
  "devDependencies": { "jest": "^29.0.0" }
}
PKGJSON

  # Create a dummy test file so has_tests=true
  mkdir -p "$TEST_ROOT/src/__tests__"
  touch "$TEST_ROOT/src/__tests__/app.test.js"

  # Default detection JSON (has_tests=true, react, jest)
  DETECT_JSON='{"frameworks":["react"],"versions":{"react":"18.2.0"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}'
  DETECT_FILE="$TEST_ROOT/detect.json"
  printf '%s\n' "$DETECT_JSON" > "$DETECT_FILE"

  # Output dir for .autospec artifacts
  AUTOSPEC_DIR="$TEST_ROOT/.autospec"
  mkdir -p "$AUTOSPEC_DIR"

  # behavior-lock.sh intentionally resolves mutation-gate.sh from
  # AUTOSPEC_SCRIPTS_DIR before PATH. Point that resolver at the test's mock bin
  # so an operator's installed ~/.autospec/scripts/mutation-gate.sh cannot leak
  # into this no-network unit suite.
  export AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN"

  export TEST_ROOT MOCK_BIN DETECT_FILE AUTOSPEC_DIR
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Helper: install a mock binary into MOCK_BIN ───────────────────────────────

install_mock() {
  local name="$1"
  local body="$2"
  local path="$MOCK_BIN/$name"
  printf '#!/usr/bin/env bash\n%s\n' "$body" > "$path"
  chmod +x "$path"
}

# ── Existence / executability ─────────────────────────────────────────────────

@test "behavior-lock.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

# ── Locked case: mocks succeed → exit 0, golden-master + baseline written ─────

@test "locked: exit 0 when golden-master and baseline are both produced" {
  # autospec-test succeeds and writes a coverage report
  install_mock "autospec-test" 'exit 0'
  # autospec-playwright succeeds
  install_mock "autospec-playwright" 'exit 0'
  # mutation-gate --baseline succeeds and writes .autospec/mutation-proof.json
  install_mock "mutation-gate.sh" '
    # parse --out from args
    out_dir=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --out) out_dir="$2"; shift 2 ;;
        --out=*) out_dir="${1#--out=}"; shift ;;
        *) shift ;;
      esac
    done
    if [ -n "$out_dir" ]; then
      mkdir -p "$out_dir"
      printf '"'"'{"baseline":{"score":80},"recorded_at":"2026-06-18"}'"'"' > "$out_dir/mutation-proof.json"
    fi
    exit 0
  '

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

@test "locked: golden-master file exists under .autospec/ on success" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" '
    out_dir=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --out) out_dir="$2"; shift 2 ;;
        --out=*) out_dir="${1#--out=}"; shift ;;
        *) shift ;;
      esac
    done
    if [ -n "$out_dir" ]; then
      mkdir -p "$out_dir"
      printf '"'"'{"baseline":{"score":80},"recorded_at":"2026-06-18"}'"'"' > "$out_dir/mutation-proof.json"
    fi
    exit 0
  '

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"

  # The script must write a golden-master snapshot
  local gm_file
  gm_file="$AUTOSPEC_DIR/behavior-lock/golden-master.json"
  [ -f "$gm_file" ]
}

@test "locked: golden-master JSON has required fields" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" '
    out_dir=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --out) out_dir="$2"; shift 2 ;;
        --out=*) out_dir="${1#--out=}"; shift ;;
        *) shift ;;
      esac
    done
    if [ -n "$out_dir" ]; then
      mkdir -p "$out_dir"
      printf '"'"'{"baseline":{"score":80}}'"'"' > "$out_dir/mutation-proof.json"
    fi
    exit 0
  '

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"

  local gm_file="$AUTOSPEC_DIR/behavior-lock/golden-master.json"
  [ -f "$gm_file" ]
  jq -e 'has("locked_at") and has("frameworks") and has("runners")' "$gm_file"
}

# ── Missing-baseline case: golden-master ok but no baseline → exit != 0 ───────

@test "missing-baseline: exit non-zero when mutation-gate fails" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  # mutation-gate fails (baseline not recorded)
  install_mock "mutation-gate.sh" 'exit 1'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "missing-baseline: no golden-master file written when baseline fails" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" 'exit 1'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"

  local gm_file="$AUTOSPEC_DIR/behavior-lock/golden-master.json"
  # golden-master must NOT be present when baseline failed
  [ ! -f "$gm_file" ]
}

# ── Unreachable-surface case: zero tests → exit != 0 + code_health message ────

@test "unreachable: exit non-zero when has_tests is false" {
  # Detection JSON with has_tests=false (zero runnable surface)
  local zero_detect="$TEST_ROOT/detect-zero.json"
  printf '%s\n' '{"frameworks":["react"],"versions":{"react":"18.2.0"},"package_manager":"npm","runners":[],"monorepo":false,"has_tests":false}' \
    > "$zero_detect"

  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" 'exit 0'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$zero_detect" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "unreachable: output contains upgrade_behavior_lock_unreachable when has_tests false" {
  local zero_detect="$TEST_ROOT/detect-zero.json"
  printf '%s\n' '{"frameworks":["react"],"versions":{"react":"18.2.0"},"package_manager":"npm","runners":[],"monorepo":false,"has_tests":false}' \
    > "$zero_detect"

  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" 'exit 0'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$zero_detect" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  printf '%s\n' "$output" | grep -q 'upgrade_behavior_lock_unreachable'
}

@test "unreachable: autospec-test failure triggers unreachable exit" {
  install_mock "autospec-test" 'exit 1'
  install_mock "autospec-playwright" 'exit 1'
  install_mock "mutation-gate.sh" 'exit 0'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "unreachable: output contains upgrade_behavior_lock_unreachable when autospec-test fails" {
  install_mock "autospec-test" 'exit 1'
  install_mock "autospec-playwright" 'exit 1'
  install_mock "mutation-gate.sh" 'exit 0'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  printf '%s\n' "$output" | grep -q 'upgrade_behavior_lock_unreachable'
}

# ── Argument validation ────────────────────────────────────────────────────────

@test "exits non-zero when --detect file does not exist" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" 'exit 0'

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "/nonexistent/detect.json" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

# ── Idempotency: re-run with a pre-existing successful state ──────────────────

@test "locked re-run: still exits 0 (idempotent)" {
  install_mock "autospec-test" 'exit 0'
  install_mock "autospec-playwright" 'exit 0'
  install_mock "mutation-gate.sh" '
    out_dir=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --out) out_dir="$2"; shift 2 ;;
        --out=*) out_dir="${1#--out=}"; shift ;;
        *) shift ;;
      esac
    done
    if [ -n "$out_dir" ]; then
      mkdir -p "$out_dir"
      printf '"'"'{"baseline":{"score":80}}'"'"' > "$out_dir/mutation-proof.json"
    fi
    exit 0
  '

  env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"

  run env PATH="$MOCK_BIN:$PATH" \
    "$SCRIPT" --detect "$DETECT_FILE" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}
