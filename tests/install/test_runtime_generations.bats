#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_ROOT="$(mktemp -d "${BATS_TMPDIR:-/tmp}/runtime-generations.XXXXXX")"
  TEST_HOME="$TEST_ROOT/home"
  FIXTURE_REPO="$TEST_ROOT/repo"
  FAKE_BIN="$TEST_ROOT/bin"
  BUILD_LOG="$TEST_ROOT/build.log"
  mkdir -p "$TEST_HOME" "$FIXTURE_REPO/crates/demo/src" "$FAKE_BIN"
  printf '[workspace]\nmembers=["crates/demo"]\n' >"$FIXTURE_REPO/Cargo.toml"
  printf '# lock\n' >"$FIXTURE_REPO/Cargo.lock"
  printf '[package]\nname="demo"\nversion="0.1.0"\n' >"$FIXTURE_REPO/crates/demo/Cargo.toml"
  printf 'pub const BUILD: &str = "one";\n' >"$FIXTURE_REPO/crates/demo/src/lib.rs"
  git -C "$FIXTURE_REPO" init -q
  git -C "$FIXTURE_REPO" config user.email test@example.com
  git -C "$FIXTURE_REPO" config user.name 'Generation Test'
  git -C "$FIXTURE_REPO" add .
  git -C "$FIXTURE_REPO" commit -qm fixture

  cat >"$FAKE_BIN/cargo" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'build %s\n' "$PWD" >>"$AUTOSPEC_TEST_BUILD_LOG"
mkdir -p "$CARGO_TARGET_DIR/release"
cp "$PWD/crates/demo/src/lib.rs" "$CARGO_TARGET_DIR/release/autospec"
chmod +x "$CARGO_TARGET_DIR/release/autospec"
if [ -n "${AUTOSPEC_TEST_BUILD_ENTERED:-}" ]; then
  : >"$AUTOSPEC_TEST_BUILD_ENTERED"
  while [ ! -e "$AUTOSPEC_TEST_BUILD_RELEASE" ]; do sleep 0.02; done
fi
if [ "${AUTOSPEC_TEST_MUTATE_SOURCE:-0}" = 1 ]; then
  printf '// moved\n' >>"$PWD/crates/demo/src/lib.rs"
fi
SH
  chmod +x "$FAKE_BIN/cargo"
}

teardown() {
  find "$TEST_ROOT" -type d -exec chmod u+w {} + 2>/dev/null || true
  rm -rf "$TEST_ROOT"
}

install_runtime() {
  run env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
}

mode_of() {
  stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1"
}

wait_for_file() {
  file=$1
  waiting_pid=$2
  attempts=0
  while [ ! -e "$file" ] && kill -0 "$waiting_pid" 2>/dev/null && [ "$attempts" -lt 250 ]; do
    sleep 0.02
    attempts=$((attempts + 1))
  done
  [ -e "$file" ]
}

@test "installer publishes an immutable digest generation and prints its exact executable" {
  install_runtime
  [ "$status" -eq 0 ]
  generation_binary="$output"
  [ -x "$generation_binary" ]
  digest="$(basename "$(dirname "$generation_binary")")"
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]]
  [ "$generation_binary" = "$TEST_HOME/.autospec/runtime-generations/$digest/autospec" ]
  [ -f "$(dirname "$generation_binary")/receipt" ]
  [ "$(mode_of "$TEST_HOME/.autospec")" = 700 ]
  [ "$(mode_of "$(dirname "$generation_binary")")" = 500 ]
  [ "$(mode_of "$(dirname "$generation_binary")/receipt")" = 600 ]
  [ "$(readlink "$TEST_HOME/.autospec/runtime-generations/current")" = "$digest" ]

  before="$(shasum -a 256 "$generation_binary" | awk '{print $1}')"
  install_runtime
  [ "$status" -eq 0 ]
  [ "$output" = "$generation_binary" ]
  [ "$(wc -l <"$BUILD_LOG" | tr -d ' ')" -eq 1 ]
  [ "$(shasum -a 256 "$generation_binary" | awk '{print $1}')" = "$before" ]
}

@test "installer rejects a moving source and retains the prior current generation" {
  install_runtime
  [ "$status" -eq 0 ]
  prior="$output"
  printf 'pub const BUILD: &str = "two";\n' >"$FIXTURE_REPO/crates/demo/src/lib.rs"

  run env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_MUTATE_SOURCE=1 bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
  [ "$status" -ne 0 ]
  [ "$(readlink "$TEST_HOME/.autospec/runtime-generations/current")" = "$(basename "$(dirname "$prior")")" ]
  [ -x "$prior" ]
}

@test "concurrent repositories each receive the exact generation they built" {
  OTHER_REPO="$TEST_ROOT/other"
  cp -R "$FIXTURE_REPO" "$OTHER_REPO"
  git -C "$OTHER_REPO" config user.email test@example.com
  printf 'pub const BUILD: &str = "other";\n' >"$OTHER_REPO/crates/demo/src/lib.rs"

  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/one.path" &
  one_pid=$!
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$OTHER_REPO" >"$TEST_ROOT/two.path" &
  two_pid=$!
  wait "$one_pid"
  wait "$two_pid"

  one_path="$(cat "$TEST_ROOT/one.path")"
  two_path="$(cat "$TEST_ROOT/two.path")"
  [ "$one_path" != "$two_path" ]
  grep -q 'one' "$one_path"
  grep -q 'other' "$two_path"
}

@test "signal cleanup and SIGKILL recovery never expose a partial generation" {
  entered="$TEST_ROOT/entered"
  release="$TEST_ROOT/release"
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_BUILD_ENTERED="$entered" AUTOSPEC_TEST_BUILD_RELEASE="$release" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/path" &
  pid=$!
  wait_for_file "$entered" "$pid"
  kill -TERM "$pid"
  touch "$release"
  wait "$pid" || true
  [ ! -e "$TEST_HOME/.autospec/runtime-install.lock" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.transaction" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-generations/current" ]

  rm -f "$entered" "$release"
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_BUILD_ENTERED="$entered" AUTOSPEC_TEST_BUILD_RELEASE="$release" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/path" &
  pid=$!
  wait_for_file "$entered" "$pid"
  kill -KILL "$pid"
  touch "$release"
  wait "$pid" || true

  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.lock" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.transaction" ]
}

@test "lock metadata rejects ambiguity but reclaims a reused process identity" {
  lock="$TEST_HOME/.autospec/runtime-install.lock"
  mkdir -p "$lock"
  chmod 700 "$TEST_HOME/.autospec" "$lock"
  printf 'not metadata\n' >"$lock/owner"
  chmod 600 "$lock/owner"
  install_runtime
  [ "$status" -ne 0 ]

  rm -rf "$lock"
  mkdir "$lock"
  chmod 700 "$lock"
  printf 'pid=%s\nstart=definitely-not-this-process\ncreated_at=2026-08-13T00:00:00Z\n' "$$" >"$lock/owner"
  chmod 600 "$lock/owner"
  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]
}

@test "top-level installer delegates runtime publication to the narrow installer" {
  run env HOME="$TEST_HOME" AUTOSPEC_SKIP_SYSTEM_TOOLS=1 AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_SUPERPOWERS=1 AUTOSPEC_SKIP_OH_MY_CODEX=1 AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
    AUTOSPEC_SKIP_OH_MY_CLAUDE=1 AUTOSPEC_NO_STAR_PROMPT=1 CI=1 \
    bash "$REPO_ROOT/install.sh" --skill autospec-autonomous --harness codex --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-runtime-install.sh --repo-dir"* ]]
}
