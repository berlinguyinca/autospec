#!/usr/bin/env bats
#
# Guards start_detached()'s repo-dir resolution + fail-loud gate.
#
# Bug: with AUTOSPEC_REPO_DIR unset and no --repo-dir, the launcher fell back to
# DEFAULT_REPO_DIR (=$SCRIPT_DIR/.. = ~/.autospec when installed), so the conductor
# analyzed installed script copies instead of the real checkout — silently.
#
# All start cases stub python3 on PATH so the child conductor is never spawned;
# the stub records the resolved repo_dir (its 4th argument) and prints a fake pid.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CLI="$REPO_ROOT/scripts/autospec-autonomous.sh"
  TEST_TMP="$(mktemp -d)"
  export HOME="$TEST_TMP/home"
  mkdir -p "$HOME"
  export AUTOSPEC_AUTONOMOUS_OPERATOR_DIR="$TEST_TMP/operator"
  unset AUTOSPEC_REPO_DIR
  unset CONDUCTOR_REPO
  unset AUTOSPEC_STOP_FLAG_FILE

  # python3 stub: capture the resolved repo_dir (arg 4) and emit a fake pid so
  # start_detached records metadata without launching a real conductor.
  export CAPTURE_FILE="$TEST_TMP/captured-repo-dir"
  mkdir -p "$TEST_TMP/bin"
  cat > "$TEST_TMP/bin/python3" <<'PY_STUB'
#!/usr/bin/env bash
# args: - <script> <log> <repo_dir> <scripts_dir> <repo>
printf '%s\n' "$4" > "$CAPTURE_FILE"
printf '987654\n'
PY_STUB
  chmod +x "$TEST_TMP/bin/python3"
  export PATH="$TEST_TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TEST_TMP"
}

# Create a git checkout at $1 with origin remote $2 (optional).
make_git_repo() {
  local dir="$1" origin="${2:-}"
  mkdir -p "$dir"
  git -C "$dir" init -q
  if [ -n "$origin" ]; then
    git -C "$dir" remote add origin "$origin"
  fi
}

@test "start: unset AUTOSPEC_REPO_DIR resolves to the launch cwd git top-level, not ~/.autospec" {
  local checkout="$TEST_TMP/checkout"
  make_git_repo "$checkout" "https://github.com/acme/widget.git"

  run bash -c "cd '$checkout' && bash '$CLI' start --force"

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous started"* ]]
  # git top-level may canonicalize symlinks (macOS /tmp); compare realpaths.
  local captured expected
  captured="$(cd "$(cat "$CAPTURE_FILE")" && pwd -P)"
  expected="$(cd "$checkout" && pwd -P)"
  [ "$captured" = "$expected" ]
}

@test "start: explicit --repo-dir wins over cwd and default" {
  local elsewhere="$TEST_TMP/elsewhere"
  local target="$TEST_TMP/target"
  make_git_repo "$elsewhere"
  make_git_repo "$target" "https://github.com/acme/widget.git"

  run bash -c "cd '$elsewhere' && bash '$CLI' start --repo-dir '$target' --force"

  [ "$status" -eq 0 ]
  local captured expected
  captured="$(cd "$(cat "$CAPTURE_FILE")" && pwd -P)"
  expected="$(cd "$target" && pwd -P)"
  [ "$captured" = "$expected" ]
}

@test "start: resolved repo dir that is not a git checkout fails loud and does not spawn" {
  local nongit="$TEST_TMP/nongit"
  mkdir -p "$nongit"
  rm -f "$CAPTURE_FILE"

  run bash "$CLI" start --repo-dir "$nongit" --force

  [ "$status" -ne 0 ]
  [[ "$output" == *"not a git checkout"* ]]
  [[ "$output" == *"--repo-dir"* ]]
  # never reached the python3 spawn stub
  [ ! -f "$CAPTURE_FILE" ]
}

@test "start: reported invocation — --repo matching slug, no --repo-dir, from inside the checkout resolves to cwd with no warning" {
  local checkout="$TEST_TMP/checkout"
  make_git_repo "$checkout" "https://github.com/acme/widget.git"

  run bash -c "cd '$checkout' && bash '$CLI' start --repo acme/widget --force"

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous started"* ]]
  [[ "$output" != *"warning"* ]]
  local captured expected
  captured="$(cd "$(cat "$CAPTURE_FILE")" && pwd -P)"
  expected="$(cd "$checkout" && pwd -P)"
  [ "$captured" = "$expected" ]
}

@test "start: --repo slug not in checkout origin warns but still launches" {
  local checkout="$TEST_TMP/checkout"
  make_git_repo "$checkout" "https://github.com/acme/widget.git"

  run bash "$CLI" start --repo-dir "$checkout" --repo other-owner/other-repo --force

  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-autonomous started"* ]]
  [[ "$output" == *"warning"* ]]
  [[ "$output" == *"other-owner/other-repo"* ]]
  [ -f "$CAPTURE_FILE" ]
}
