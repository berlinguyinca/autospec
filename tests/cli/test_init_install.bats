#!/usr/bin/env bats
# tests/cli/test_init_install.bats — bats coverage for autospec CLI init + install subcommands

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
CLI_BIN="$REPO_ROOT/packages/cli/bin/autospec.js"

setup() {
  TMPDIR_WORK="$(mktemp -d)"
}

teardown() {
  rm -rf "$TMPDIR_WORK"
}

@test "autospec init creates .autospec/test.yml in cwd" {
  run bash -c "cd '$TMPDIR_WORK' && node '$CLI_BIN' init"
  [ "$status" -eq 0 ]
  [ -f "$TMPDIR_WORK/.autospec/test.yml" ]
}

@test "autospec install exits 0 against repo root" {
  # install subcommand should invoke install.sh with --help or dry-run and exit 0
  run node "$CLI_BIN" install --dry-run
  [ "$status" -eq 0 ]
}

@test "autospec --version prints package version" {
  run node "$CLI_BIN" --version
  [ "$status" -eq 0 ]
  [[ "$output" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]
}

@test "CLI dispatcher has no debug console logging" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|debugger' "$CLI_BIN"
}
