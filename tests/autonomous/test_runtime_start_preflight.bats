#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LAUNCHER_SOURCE="$REPO_ROOT/scripts/autospec-autonomous-launcher.sh"
  TEST_ROOT="$(mktemp -d)"
  export HOME="$TEST_ROOT/home"
  export PATH="$TEST_ROOT/bin:/usr/bin:/bin"
  mkdir -p "$HOME" "$TEST_ROOT/bin" "$TEST_ROOT/repo/.git" "$TEST_ROOT/runtime" "$TEST_ROOT/scripts"
  printf '%s\n' '#!/usr/bin/env bash' 'printf "%s\n" "$@" > "$AUTOSPEC_TEST_GENERATION_ARGS"' \
    > "$TEST_ROOT/runtime/autospec"
  chmod +x "$TEST_ROOT/runtime/autospec"
  export AUTOSPEC_TEST_GENERATION_ARGS="$TEST_ROOT/generation.args"
  export AUTOSPEC_TEST_OLD_ARGS="$TEST_ROOT/old.args"
  export AUTOSPEC_TEST_HELPER_CALLS="$TEST_ROOT/helper.calls"
  export AUTOSPEC_TEST_STATUS_COUNT="$TEST_ROOT/status.count"
  export AUTOSPEC_TEST_GENERATION="$TEST_ROOT/runtime/autospec"

  cat > "$TEST_ROOT/bin/git" <<'EOF'
#!/usr/bin/env bash
if [ "${1-}" = rev-parse ] && [ "${2-}" = --show-toplevel ]; then
  printf '%s\n' "$AUTOSPEC_TEST_REPO"
  exit 0
fi
exec /usr/bin/git "$@"
EOF
  chmod +x "$TEST_ROOT/bin/git"
  export AUTOSPEC_TEST_REPO="$TEST_ROOT/repo"

  cat > "$TEST_ROOT/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" >> "$AUTOSPEC_TEST_OLD_ARGS"
case "${1-}:${2-}" in
  autonomous:status)
    count=0
    [ ! -f "$AUTOSPEC_TEST_STATUS_COUNT" ] || count="$(cat "$AUTOSPEC_TEST_STATUS_COUNT")"
    count=$((count + 1))
    printf '%s\n' "$count" > "$AUTOSPEC_TEST_STATUS_COUNT"
    if [ "$count" -le "${AUTOSPEC_TEST_RUNNING_POLLS:-0}" ]; then
      printf '%s\n' '{"running":true}'
    else
      printf '%s\n' '{"running":false}'
    fi
    ;;
esac
EOF
  chmod +x "$TEST_ROOT/bin/autospec"

  cat > "$TEST_ROOT/scripts/autonomous-runtime-refresh.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$AUTOSPEC_TEST_HELPER_CALLS"
case "$1" in
  check)
    if [ "${AUTOSPEC_TEST_STALE:-0}" = 1 ]; then
      printf '%s\n' 'stale:source-digest'
      exit 10
    fi
    printf '%s\n' "$AUTOSPEC_TEST_GENERATION"
    ;;
  ensure)
    [ "${AUTOSPEC_TEST_BUILD_FAIL:-0}" != 1 ] || exit 2
    printf '%s\n' "$AUTOSPEC_TEST_GENERATION"
    ;;
esac
EOF
  chmod +x "$TEST_ROOT/scripts/autonomous-runtime-refresh.sh"
}

teardown() {
  rm -rf "$TEST_ROOT"
}

install_launcher_fixture() {
  cp "$LAUNCHER_SOURCE" "$TEST_ROOT/scripts/autospec-autonomous-launcher.sh"
  chmod +x "$TEST_ROOT/scripts/autospec-autonomous-launcher.sh"
  LAUNCHER="$TEST_ROOT/scripts/autospec-autonomous-launcher.sh"
}

@test "current start bypasses rebuild and executes the verified generation" {
  install_launcher_fixture

  run "$LAUNCHER" start --repo-dir "$TEST_ROOT/repo" --follow

  [ "$status" -eq 0 ]
  [ "$(cat "$AUTOSPEC_TEST_HELPER_CALLS")" = check ]
  [ "$(sed -n '1p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = autonomous ]
  [ "$(sed -n '2p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = start ]
  [ "$(sed -n '3p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = --repo-dir ]
  [ "$(sed -n '4p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = "$TEST_ROOT/repo" ]
  [ "$(sed -n '5p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = --follow ]
}

@test "start fails closed when the runtime refresh helper is missing" {
  cp "$LAUNCHER_SOURCE" "$TEST_ROOT/scripts/autospec-autonomous-launcher.sh"
  chmod +x "$TEST_ROOT/scripts/autospec-autonomous-launcher.sh"
  rm "$TEST_ROOT/scripts/autonomous-runtime-refresh.sh"

  run "$TEST_ROOT/scripts/autospec-autonomous-launcher.sh" start --repo-dir "$TEST_ROOT/repo"

  [ "$status" -ne 0 ]
  [[ "$output" == *"runtime refresh helper is required"* ]]
  [ ! -e "$AUTOSPEC_TEST_GENERATION_ARGS" ]
  [ ! -e "$AUTOSPEC_TEST_OLD_ARGS" ]
}

@test "stale stopped start rebuilds without requesting a stop" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1

  run "$LAUNCHER" start --repo-dir "$TEST_ROOT/repo" --json

  [ "$status" -eq 0 ]
  [ "$(tr '\n' ' ' < "$AUTOSPEC_TEST_HELPER_CALLS")" = "check ensure " ]
  run grep -q '^stop$' "$AUTOSPEC_TEST_OLD_ARGS"
  [ "$status" -ne 0 ]
  [ "$(sed -n '2p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = start ]
}

@test "stale live start requests graceful drain before rebuilding" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1
  export AUTOSPEC_TEST_RUNNING_POLLS=2

  run "$LAUNCHER" start --repo-dir "$TEST_ROOT/repo" --follow

  [ "$status" -eq 0 ]
  grep -q '^stop$' "$AUTOSPEC_TEST_OLD_ARGS"
  grep -q '^--graceful$' "$AUTOSPEC_TEST_OLD_ARGS"
  [ "$(cat "$AUTOSPEC_TEST_STATUS_COUNT")" -ge 3 ]
  [ "$(tail -n 1 "$AUTOSPEC_TEST_HELPER_CALLS")" = ensure ]
}

@test "stale live start times out without rebuilding or launching" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1
  export AUTOSPEC_TEST_RUNNING_POLLS=9999
  cat > "$TEST_ROOT/bin/sleep" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  chmod +x "$TEST_ROOT/bin/sleep"

  run "$LAUNCHER" start --repo-dir "$TEST_ROOT/repo"

  [ "$status" -ne 0 ]
  [[ "$output" == *"timed out waiting for the stale autonomous scope to stop"* ]]
  [ "$(grep -c '^ensure$' "$AUTOSPEC_TEST_HELPER_CALLS" || true)" -eq 0 ]
  [ ! -e "$AUTOSPEC_TEST_GENERATION_ARGS" ]
}

@test "failed rebuild retains the prior runtime and does not launch stale work" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1
  export AUTOSPEC_TEST_BUILD_FAIL=1

  run "$LAUNCHER" restart --repo-dir "$TEST_ROOT/repo"

  [ "$status" -ne 0 ]
  [[ "$output" == *"could not publish a fresh autonomous runtime"* ]]
  [ ! -e "$AUTOSPEC_TEST_GENERATION_ARGS" ]
}

@test "read-only and stop commands bypass the refresh helper" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1

  run "$LAUNCHER" status --repo-dir "$TEST_ROOT/repo" --json
  [ "$status" -eq 0 ]
  run "$LAUNCHER" stop --repo-dir "$TEST_ROOT/repo" --graceful
  [ "$status" -eq 0 ]

  [ ! -e "$AUTOSPEC_TEST_HELPER_CALLS" ]
  [ "$(grep -c '^autonomous$' "$AUTOSPEC_TEST_OLD_ARGS")" -eq 2 ]
}

@test "ambiguous stale scope metadata fails closed before rebuild" {
  install_launcher_fixture
  export AUTOSPEC_TEST_STALE=1
  cat > "$TEST_ROOT/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"metadata_state":"ambiguous"}'
EOF
  chmod +x "$TEST_ROOT/bin/autospec"

  run "$LAUNCHER" start --repo-dir "$TEST_ROOT/repo"

  [ "$status" -ne 0 ]
  [[ "$output" == *"cannot verify whether the stale autonomous scope is stopped"* ]]
  [ "$(grep -c '^ensure$' "$AUTOSPEC_TEST_HELPER_CALLS" || true)" -eq 0 ]
  [ ! -e "$AUTOSPEC_TEST_GENERATION_ARGS" ]
}

@test "generated wrappers delegate minimally to the shared launcher" {
  local wrapper="$TEST_ROOT/bin/autospec-autonomous-start"
  info() { :; }
  warn() { :; }
  DRY_RUN=0
  # shellcheck source=scripts/lib/install-operator-wrappers.sh
  source "$REPO_ROOT/scripts/lib/install-operator-wrappers.sh"

  write_autonomous_operator_wrapper "$wrapper" start

  grep -qF 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous-launcher.sh" start "$@"' "$wrapper"
  [ "$(grep -c '^exec ' "$wrapper")" -eq 1 ]
}

@test "bare start resolves the caller git root and preserves option arguments" {
  install_launcher_fixture

  run bash -c 'cd "$1" && exec "$2" --repo owner/repo --max-cycles 7' _ \
    "$TEST_ROOT/repo" "$LAUNCHER"

  [ "$status" -eq 0 ]
  [ "$(sed -n '2p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = --repo ]
  [ "$(sed -n '3p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = owner/repo ]
  [ "$(sed -n '4p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = --max-cycles ]
  [ "$(sed -n '5p' "$AUTOSPEC_TEST_GENERATION_ARGS")" = 7 ]
}
