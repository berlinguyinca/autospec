#!/usr/bin/env bats
# tests/portable-stat-helpers.bats — stat(1) portability across GNU and BSD.
#
# GNU coreutils `stat -f FMT FILE` means "filesystem status": it treats FMT as a
# file operand (erroring to stderr) but STILL prints a multi-line filesystem
# report for FILE to stdout, then exits non-zero. A `stat -f ... || stat -c ...`
# fallback chain therefore emits the filesystem report AND the real value, so
# every caller that captures the chain gets unusable multi-line output.
#
# The portable order is GNU first: BSD stat has no `-c`, so it fails cleanly to
# stderr with no stdout, and the `-f` fallback then supplies the value.

REFRESH="${BATS_TEST_DIRNAME}/../scripts/autonomous-runtime-refresh.sh"

setup() {
  TMP="$(mktemp -d)"
  chmod 700 "$TMP"
}

teardown() {
  rm -rf "$TMP"
}

@test "autospec_runtime_stat_mode emits a single bare octal mode" {
  run bash -c "source '$REFRESH'; autospec_runtime_stat_mode '$TMP'"
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 1 ]
  [[ "${lines[0]}" =~ ^[0-7]{3,4}$ ]]
  [ "${lines[0]}" = "700" ]
}

@test "autospec_runtime_stat_owner emits a single bare numeric uid" {
  run bash -c "source '$REFRESH'; autospec_runtime_stat_owner '$TMP'"
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 1 ]
  [[ "${lines[0]}" =~ ^[0-9]+$ ]]
  [ "${lines[0]}" = "$(id -u)" ]
}

@test "autospec_runtime_private_dir accepts a 0700 dir owned by the caller" {
  run bash -c "source '$REFRESH'; autospec_runtime_private_dir '$TMP'"
  [ "$status" -eq 0 ]
}

@test "autospec_runtime_private_dir rejects a group/world-readable dir" {
  chmod 755 "$TMP"
  run bash -c "source '$REFRESH'; autospec_runtime_private_dir '$TMP'"
  [ "$status" -ne 0 ]
}

@test "no tracked shell script falls back from stat -f to stat -c" {
  # `git grep` exits 1 when nothing matches, which is the passing case here.
  run git -C "${BATS_TEST_DIRNAME}/.." grep -nE 'stat[[:space:]]+-f[^|]*\|\|[[:space:]]*stat[[:space:]]+-c' -- '*.sh'
  [ "$status" -eq 1 ]
  [ "$output" = "" ]
}
